//! The synthesizer. Each device is modelled from how it physically makes
//! sound:
//!
//! Keypad sounder: a piezo disc is a sharp mechanical resonator. It's driven
//! with a square wave and rings at its own frequency, so tone, attack and
//! decay come from the resonance. The PC1500RK's older disc is lower, broader
//! and buzzier; the PK5500's is higher and cleaner. DSC doesn't publish the
//! frequencies; these are typical of keypad piezos (2.3 to 3.3 kHz).
//!
//! Ademco WAVE-2F: an electronic siren driver into a small horn speaker. Its
//! steady output on terminal 1 is a repeating rising sweep, its warble on
//! terminal 2 alternates two tones. The speaker and its cavity colour it.
//!
//! Klaxon Master Blaster: a motor spins a slotted rotor inside a stator and
//! chops the air. Pitch is rotor speed (1600 Hz at full speed), so it winds
//! up when powered and coasts down when the bell output drops.
//!
//! ADT RX-7C: a small self contained piezo sounder that chirps.
//!
//! Motor bell: a striker hits a steel gong many times a second; each strike
//! excites the gong's inharmonic vibration modes, which ring and decay.

use std::collections::VecDeque;

use super::{AudioCmd, BellPattern, ChimeKind, KeypadVoice, Pattern, ToneSeg};
use crate::engine::types::SirenModel;

const TAU: f32 = std::f32::consts::TAU;
const PI: f32 = std::f32::consts::PI;

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
        let y = self.a1 * self.y1 + self.a2 * self.y2 + self.b * x;
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
        OnePole { a: 1.0 - (-TAU * f / sr).exp(), z: 0.0 }
    }
    fn lowpass(&mut self, x: f32) -> f32 {
        self.z += self.a * (x - self.z);
        self.z
    }
    fn highpass(&mut self, x: f32) -> f32 {
        x - self.lowpass(x)
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

/// Band limited square wave.
fn square(phase: f32, dt: f32) -> f32 {
    let mut v = if phase < 0.5 { 1.0 } else { -1.0 };
    v += blep(phase, dt);
    let mut p2 = phase + 0.5;
    if p2 >= 1.0 {
        p2 -= 1.0;
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
    fn next(&mut self, sr: f32) -> bool {
        match self.pat {
            Pattern::Off => false,
            Pattern::Steady => true,
            Pattern::Beeps { on_ms, off_ms, count, period_ms } => {
                let on = ms(sr, on_ms as u32).max(1);
                let unit = on + ms(sr, off_ms as u32);
                let period = ms(sr, period_ms).max(unit * count.max(1) as u32).max(1);
                let p = self.pos;
                self.pos = (self.pos + 1) % period;
                (p / unit) < count as u32 && (p % unit) < on
            }
        }
    }
}

/// The keypad piezo.
struct Piezo {
    res: Res,
    res2: Res,
    phase: f32,
    f0: f32,
    buzz: f32,
    sat: f32,
    second: f32,
    sr: f32,
}

impl Piezo {
    fn new(sr: f32, voice: KeypadVoice) -> Piezo {
        let (f0, q, buzz, sat, second) = match voice {
            KeypadVoice::Classic => (2850.0, 9.0, 0.32, 2.2, 0.35),
            KeypadVoice::Pk5500 => (3150.0, 16.0, 0.12, 1.5, 0.0),
        };
        Piezo { res: Res::new(sr, f0, q), res2: Res::new(sr, f0 * 1.021, q * 0.8), phase: 0.0, f0, buzz, sat, second, sr }
    }
    fn next(&mut self, on: bool, freq: f32, amp: f32) -> f32 {
        let drive = if on && amp > 0.0 {
            self.phase += freq / self.sr;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            ((TAU * self.phase).sin() * 5.0).tanh() * amp
        } else {
            0.0
        };
        let y = self.res.tick(drive) + self.second * self.res2.tick(drive);
        // Tones away from the disc's resonance (chime notes) are carried by the drive itself.
        let off = ((freq - self.f0).abs() / self.f0 * 2.5).min(0.8);
        let v = y * (1.0 - self.buzz) + drive * (self.buzz + off) * 0.4;
        (v * self.sat).tanh() / self.sat.tanh()
    }
}

/// All four bell output devices.
struct Siren {
    model: SirenModel,
    warble: bool,
    sr: f32,
    phase: f32,
    sweep: f32,
    glide: f32,
    amp: f32,
    hp: OnePole,
    spk: Res,
    cab: Res,
    w: f32,
    theta: f32,
    air: Res,
    hum: f32,
    count: u32,
    lp: OnePole,
    rx_t: f32,
    rx_res: Res,
    modes: [Res; 5],
    weights: [f32; 5],
    strike: f32,
    rng: Rng,
}

impl Siren {
    fn new(sr: f32, model: SirenModel, warble: bool) -> Siren {
        let mode = |f: f32, decay_s: f32| Res::new(sr, f, PI * f * decay_s);
        let (hp_f, lp_f) = match model {
            SirenModel::MasterBlaster => (140.0, 6500.0),
            _ => (350.0, 9000.0),
        };
        Siren {
            model,
            warble,
            sr,
            phase: 0.0,
            sweep: 0.0,
            glide: 0.0,
            amp: 0.0,
            hp: OnePole::new(sr, hp_f),
            spk: Res::new(sr, 2400.0, 1.6),
            cab: Res::new(sr, 900.0, 2.2),
            w: 0.0,
            theta: 0.0,
            air: Res::new(sr, 1600.0, 4.0),
            hum: 0.0,
            count: 0,
            lp: OnePole::new(sr, lp_f),
            rx_t: 0.0,
            rx_res: Res::new(sr, 3300.0, 8.0),
            modes: [mode(510.0, 1.6), mode(1270.0, 1.0), mode(2110.0, 0.65), mode(3020.0, 0.45), mode(3890.0, 0.3)],
            weights: [1.0, 0.7, 0.55, 0.35, 0.2],
            strike: 0.0,
            rng: Rng(0x2545_F491),
        }
    }

    fn quiet(&self) -> bool {
        match self.model {
            SirenModel::MasterBlaster => self.w < 0.001,
            SirenModel::MotorBell => self.w < 0.01 && self.modes.iter().all(|m| m.energy() < 1e-4),
            _ => self.amp < 1e-4 && self.spk.energy() < 1e-4,
        }
    }

    fn next(&mut self, power: bool) -> f32 {
        match self.model {
            SirenModel::Wave2F => self.wave(power),
            SirenModel::MasterBlaster => self.motor(power),
            SirenModel::Rx7c => self.rx7c(power),
            SirenModel::MotorBell => self.bell(power),
        }
    }

    fn wave(&mut self, power: bool) -> f32 {
        let target = if power { 1.0 } else { 0.0 };
        let rate = if power { 0.004 } else { 0.0025 };
        self.amp += (target - self.amp) * rate;
        if !power && self.amp < 1e-4 {
            self.sweep = 0.0;
            return self.spk.tick(0.0) * 0.5;
        }
        let f = if self.warble {
            self.sweep += 3.4 / self.sr;
            if self.sweep >= 1.0 {
                self.sweep -= 1.0;
            }
            let hi = if self.sweep < 0.5 { 1.0 } else { 0.0 };
            self.glide += (hi - self.glide) * 0.0025;
            1180.0 + 520.0 * self.glide
        } else {
            self.sweep += 3.1 / self.sr;
            if self.sweep >= 1.0 {
                self.sweep -= 1.0;
            }
            let k = (1.0 - (-3.2 * self.sweep).exp()) / (1.0 - (-3.2f32).exp());
            760.0 + 980.0 * k
        };
        let dt = f / self.sr;
        self.phase += dt;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let x = self.hp.highpass(square(self.phase, dt) * self.amp);
        let y = x * 0.55 + self.spk.tick(x) * 0.9 + self.cab.tick(x) * 0.35;
        (self.lp.lowpass(y) * 1.4).tanh() * 0.9
    }

    fn motor(&mut self, power: bool) -> f32 {
        let dt = 1.0 / self.sr;
        if power {
            self.w += (1.0 - self.w) * 2.3 * dt;
        } else {
            self.w -= (self.w * 0.42 + 0.015) * dt;
            if self.w < 0.0 {
                self.w = 0.0;
            }
        }
        if self.w < 0.001 {
            return 0.0;
        }
        let f = 1600.0 * self.w;
        self.theta += f * dt;
        if self.theta >= 1.0 {
            self.theta -= 1.0;
        }
        let tone = ((TAU * self.theta).sin() * 2.6).tanh() * 0.85 + (TAU * 2.0 * self.theta).sin() * 0.12;
        self.count = self.count.wrapping_add(1);
        if self.count % 64 == 0 {
            self.air.tune(self.sr, f.max(80.0), 4.0);
        }
        let noise = self.air.tick(self.rng.next()) * 2.5;
        self.hum += 100.0 * dt;
        if self.hum >= 1.0 {
            self.hum -= 1.0;
        }
        let hum = (TAU * self.hum).sin() * 0.05 * self.w;
        let y = (tone + noise * 0.35) * self.w.powf(1.5) + hum;
        let y = self.lp.lowpass(self.hp.highpass(y));
        (y * 1.2).tanh()
    }

    fn rx7c(&mut self, power: bool) -> f32 {
        let target = if power { 1.0 } else { 0.0 };
        self.amp += (target - self.amp) * 0.01;
        if !power && self.amp < 1e-4 {
            self.rx_t = 0.0;
            return self.rx_res.tick(0.0);
        }
        let cycle = 0.075;
        self.rx_t += 1.0 / self.sr;
        if self.rx_t >= cycle {
            self.rx_t -= cycle;
        }
        let chirp = 0.052;
        let k = (self.rx_t / chirp).min(1.0);
        let env = if self.rx_t < chirp { 1.0 - k * 0.35 } else { 0.0 };
        let f = 2750.0 + 950.0 * k * k;
        self.phase += f / self.sr;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        let drive = ((TAU * self.phase).sin() * 4.0).tanh() * env * self.amp;
        let y = self.rx_res.tick(drive) + drive * 0.25;
        (y * 1.6).tanh() * 0.8
    }

    fn bell(&mut self, power: bool) -> f32 {
        let dt = 1.0 / self.sr;
        if power {
            self.w += (1.0 - self.w) * 9.0 * dt;
        } else {
            self.w -= self.w * 5.0 * dt;
        }
        if self.w > 0.08 {
            self.strike += 17.0 * self.w * dt;
            if self.strike >= 1.0 {
                self.strike -= 1.0;
                let a = (0.6 + 0.4 * self.w) * (0.9 + 0.1 * self.rng.next());
                for (m, wgt) in self.modes.iter_mut().zip(self.weights.iter()) {
                    m.strike(a * wgt);
                }
            }
        }
        let mut y = 0.0;
        for m in self.modes.iter_mut() {
            y += m.tick(0.0);
        }
        (y * 1.3).tanh() * 0.85
    }
}

#[derive(Clone, Copy)]
struct Seg {
    on: u32,
    off: u32,
    freq: f32,
    decay: f32,
    amp: f32,
}

pub struct Synth {
    sr: f32,
    voice: KeypadVoice,
    piezo: Piezo,
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
}

impl Synth {
    pub fn new(sr: f32) -> Synth {
        let sr = sr.max(8000.0);
        Synth {
            sr,
            voice: KeypadVoice::Pk5500,
            piezo: Piezo::new(sr, KeypadVoice::Pk5500),
            sounder: Gate { pat: Pattern::Off, pos: 0 },
            segs: VecDeque::new(),
            seg: None,
            bell: BellPattern::Off,
            bell_pos: 0,
            squawks: VecDeque::new(),
            siren: Siren::new(sr, SirenModel::Wave2F, false),
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
        }
    }

    fn decay_for(&self, tau_s: f32) -> f32 {
        (-1.0 / (tau_s * self.sr)).exp()
    }

    fn push_chime(&mut self, k: ChimeKind) {
        let f0 = self.piezo.f0;
        let sr = self.sr;
        match k {
            ChimeKind::Beeps(n) => {
                for _ in 0..n.max(1) {
                    self.segs.push_back(Seg { on: ms(sr, 70), off: ms(sr, 70), freq: f0, decay: 1.0, amp: 1.0 });
                }
            }
            ChimeKind::BingBing => {
                let d = self.decay_for(0.22);
                for _ in 0..2 {
                    self.segs.push_back(Seg { on: ms(sr, 260), off: ms(sr, 110), freq: 2600.0, decay: d, amp: 1.0 });
                }
            }
            ChimeKind::DingDong => {
                let d1 = self.decay_for(0.30);
                let d2 = self.decay_for(0.42);
                self.segs.push_back(Seg { on: ms(sr, 380), off: ms(sr, 40), freq: 2600.0, decay: d1, amp: 1.0 });
                self.segs.push_back(Seg { on: ms(sr, 650), off: 0, freq: 2050.0, decay: d2, amp: 1.0 });
            }
            ChimeKind::AlarmTone => {
                for i in 0..12 {
                    let f = if i % 2 == 0 { 3000.0 } else { 2400.0 };
                    self.segs.push_back(Seg { on: ms(sr, 55), off: 0, freq: f, decay: 1.0, amp: 1.0 });
                }
            }
        }
    }

    pub fn apply(&mut self, cmd: AudioCmd) {
        match cmd {
            AudioCmd::Voice(v) => {
                if v != self.voice {
                    self.voice = v;
                    self.piezo = Piezo::new(self.sr, v);
                }
            }
            AudioCmd::Sounder(p) => self.sounder.set(p),
            AudioCmd::Beeps { count, on_ms, off_ms } => {
                // One sounder per keypad: a new beep cuts off the last one.
                self.segs.clear();
                self.seg = None;
                let f = self.piezo.f0;
                for _ in 0..count.max(1) {
                    self.segs.push_back(Seg {
                        on: ms(self.sr, on_ms as u32).max(1),
                        off: ms(self.sr, off_ms as u32),
                        freq: f,
                        decay: 1.0,
                        amp: 1.0,
                    });
                }
            }
            AudioCmd::Chime(k) => {
                self.segs.clear();
                self.seg = None;
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
                    self.squawks.push_back((ms(self.sr, 60), ms(self.sr, 240)));
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
            AudioCmd::Silence => {
                self.sounder.set(Pattern::Off);
                self.segs.clear();
                self.seg = None;
                self.bell = BellPattern::Off;
                self.squawks.clear();
                self.line.clear();
                self.line_left = 0;
                self.ring_on = false;
                self.preview = None;
            }
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
                let t = pos % (4 * sec);
                t < 3 * sec && (t % sec) < sec / 2
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
        let mut on = self.sounder.next(self.sr);
        let mut freq = self.piezo.f0;
        let mut amp = 1.0;
        if let Some((gate, f, a)) = self.seg_next() {
            if gate {
                on = true;
                freq = f;
                amp = a;
            }
        }
        let keypad = self.piezo.next(on, freq, amp) * self.vol_keypad;
        let power = self.bell_gate() | self.squawk_next();
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
        let mix = (keypad * 0.8 + siren + preview + line + ring + click) * self.vol_master;
        mix.clamp(-1.0, 1.0)
    }
}
