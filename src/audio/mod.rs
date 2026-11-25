//! Sound. Every beep, chime and siren is synthesized live from a physical
//! model of the real device; nothing is a recording. The audio thread keeps
//! the current state (sounder pattern, bell, siren choice, volumes) so it can
//! rebuild the output stream if the device changes or disappears, which also
//! tells the panel when the "bell circuit" is broken.

pub mod synth;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use crate::engine::types::SirenModel;
use crate::util;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Off,
    Steady,
    Beeps { on_ms: u16, off_ms: u16, count: u8, period_ms: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BellPattern {
    Off,
    Steady,
    Pulsed,
    Temporal3,
    Temporal4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeypadVoice {
    Classic,
    Pk5500,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChimeKind {
    Beeps(u8),
    BingBing,
    DingDong,
    AlarmTone,
}

/// One piece of phone line audio: up to two tones at once.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneSeg {
    pub f1: f32,
    pub f2: f32,
    pub ms: u32,
    pub level: f32,
}

#[derive(Clone, Debug)]
pub enum AudioCmd {
    Voice(KeypadVoice),
    BuzzerLevel(u8),
    Sounder(Pattern),
    KeyBeep,
    Beeps { count: u8, on_ms: u16, off_ms: u16 },
    Chime(ChimeKind),
    Bell(BellPattern),
    Squawk(u8),
    Siren { model: SirenModel, warble: bool },
    Volumes { master: f32, siren: f32, keypad: f32, line: f32 },
    Line(Vec<ToneSeg>),
    Ring(bool),
    Click,
    Preview { model: SirenModel, warble: bool, ms: u32 },
    Silence,
}

#[derive(Clone)]
pub struct AudioHandle {
    tx: Sender<AudioCmd>,
    ok: Arc<AtomicBool>,
}

impl AudioHandle {
    pub fn send(&self, c: AudioCmd) {
        let _ = self.tx.send(c);
    }
    /// False when there's no working output device (the "bell circuit").
    pub fn is_ok(&self) -> bool {
        self.ok.load(Ordering::Relaxed)
    }
}

pub fn start() -> AudioHandle {
    let (tx, rx) = mpsc::channel::<AudioCmd>();
    let ok = Arc::new(AtomicBool::new(false));
    let ok2 = ok.clone();
    let spawned = std::thread::Builder::new().name("audio".into()).spawn(move || run(rx, ok2));
    if let Err(e) = spawned {
        util::log(&format!("could not start the audio thread: {}", e));
    }
    AudioHandle { tx, ok }
}

/// The lasting state, replayed into each new output stream.
#[derive(Default)]
struct State {
    voice: Option<AudioCmd>,
    buzzer_level: Option<AudioCmd>,
    sounder: Option<AudioCmd>,
    bell: Option<AudioCmd>,
    siren: Option<AudioCmd>,
    volumes: Option<AudioCmd>,
    ring: Option<AudioCmd>,
}

impl State {
    fn remember(&mut self, c: &AudioCmd) {
        let slot = match c {
            AudioCmd::Voice(_) => &mut self.voice,
            AudioCmd::BuzzerLevel(_) => &mut self.buzzer_level,
            AudioCmd::Sounder(_) => &mut self.sounder,
            AudioCmd::Bell(_) => &mut self.bell,
            AudioCmd::Siren { .. } => &mut self.siren,
            AudioCmd::Volumes { .. } => &mut self.volumes,
            AudioCmd::Ring(_) => &mut self.ring,
            _ => return,
        };
        *slot = Some(c.clone());
    }

    fn replay(&self) -> Vec<AudioCmd> {
        [&self.voice, &self.buzzer_level, &self.volumes, &self.siren, &self.sounder, &self.bell, &self.ring]
            .iter()
            .filter_map(|c| (*c).clone())
            .collect()
    }
}

fn run(rx: Receiver<AudioCmd>, ok: Arc<AtomicBool>) {
    let mut state = State::default();
    let mut logged = false;
    loop {
        let failed = Arc::new(AtomicBool::new(false));
        let (itx, irx) = mpsc::channel::<AudioCmd>();
        let stream = build_stream(irx, failed.clone());
        match &stream {
            Ok(_) => {
                ok.store(true, Ordering::Relaxed);
                logged = false;
                for c in state.replay() {
                    let _ = itx.send(c);
                }
            }
            Err(e) => {
                ok.store(false, Ordering::Relaxed);
                if !logged {
                    util::log(&format!("audio output unavailable: {}", e));
                    logged = true;
                }
            }
        }
        loop {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(cmd) => {
                    state.remember(&cmd);
                    if stream.is_ok() {
                        let _ = itx.send(cmd);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if stream.is_err() || failed.load(Ordering::Relaxed) {
                break;
            }
        }
        drop(stream);
        ok.store(false, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(1500));
    }
}

fn build_stream(rx: Receiver<AudioCmd>, failed: Arc<AtomicBool>) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no audio output device".to_string())?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels.max(1) as usize;
    let rate = config.sample_rate.0 as f32;
    let mut synth = synth::Synth::new(rate);
    let flag = failed.clone();
    let on_error = move |e: cpal::StreamError| {
        util::log(&format!("audio stream error: {}", e));
        flag.store(true, Ordering::Relaxed);
    };
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                while let Ok(c) = rx.try_recv() {
                    synth.apply(c);
                }
                for frame in data.chunks_mut(channels) {
                    let s = synth.next();
                    for o in frame.iter_mut() {
                        *o = s;
                    }
                }
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                while let Ok(c) = rx.try_recv() {
                    synth.apply(c);
                }
                for frame in data.chunks_mut(channels) {
                    let s = (synth.next().clamp(-1.0, 1.0) * 32767.0) as i16;
                    for o in frame.iter_mut() {
                        *o = s;
                    }
                }
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I32 => device.build_output_stream(
            &config,
            move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                while let Ok(c) = rx.try_recv() {
                    synth.apply(c);
                }
                for frame in data.chunks_mut(channels) {
                    let s = (synth.next().clamp(-1.0, 1.0) as f64 * 2_147_483_647.0) as i32;
                    for o in frame.iter_mut() {
                        *o = s;
                    }
                }
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config,
            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                while let Ok(c) = rx.try_recv() {
                    synth.apply(c);
                }
                for frame in data.chunks_mut(channels) {
                    let s = ((synth.next().clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0) as u16;
                    for o in frame.iter_mut() {
                        *o = s;
                    }
                }
            },
            on_error,
            None,
        ),
        other => return Err(format!("unsupported sample format {:?}", other)),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}
