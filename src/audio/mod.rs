//! Sound. Every beep, chime and siren is synthesized live from a physical
//! model of the real device; nothing is a recording. The audio thread keeps
//! the current state (sounder pattern, bell, siren choice, volumes) so it can
//! rebuild the output stream if the device changes or disappears, which also
//! tells the panel when the "bell circuit" is broken.

pub mod synth;
pub mod voice;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::types::SirenModel;
use crate::util;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pattern {
    Off,
    Steady,
    Beeps { on_ms: u16, off_ms: u16, count: u8, period_ms: u32 },
    /// Continuous alternating two-tone alarm (e.g. 6160V burglary "dee-doo").
    HiLo { hi_hz: u16, lo_hz: u16, step_ms: u16 },
    /// Two short beeps then a long one, over and over: the PowerSeries entry
    /// delay's "pulsing tone" in its last ten seconds, measured from a recording.
    ShortShortLong { short_ms: u16, gap_ms: u16, long_ms: u16 },
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
    Vista6160,
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
    VoiceLevel(u8),
    Speak(Vec<String>),
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
    /// One pitched keypad beep, e.g. the Vista's low 1019 Hz confirmation.
    Tone { hz: u16, on_ms: u16, off_ms: u16, delay_ms: u16 },
    Silence,
    /// The pointer is near the keypad: make sure the output is awake.
    Wake,
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
    voice_level: Option<AudioCmd>,
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
            AudioCmd::VoiceLevel(_) => &mut self.voice_level,
            AudioCmd::Sounder(_) => &mut self.sounder,
            AudioCmd::Bell(_) => &mut self.bell,
            AudioCmd::Siren { .. } => &mut self.siren,
            AudioCmd::Volumes { .. } => &mut self.volumes,
            AudioCmd::Ring(_) => &mut self.ring,
            AudioCmd::Silence => {
                // A rebuilt output must not bring back what was silenced.
                self.sounder = Some(AudioCmd::Sounder(Pattern::Off));
                self.bell = Some(AudioCmd::Bell(BellPattern::Off));
                self.ring = Some(AudioCmd::Ring(false));
                return;
            }
            _ => return,
        };
        *slot = Some(c.clone());
    }

    /// Sounds that matter if they're missed while the output reconnects.
    fn worth_replaying(c: &AudioCmd) -> bool {
        matches!(c, AudioCmd::KeyBeep | AudioCmd::Beeps { .. } | AudioCmd::Chime(_) | AudioCmd::Tone { .. } | AudioCmd::Speak(_))
    }

    fn replay(&self) -> Vec<AudioCmd> {
        [&self.voice, &self.buzzer_level, &self.voice_level, &self.volumes, &self.siren, &self.sounder, &self.bell, &self.ring]
            .iter()
            .filter_map(|c| (*c).clone())
            .collect()
    }
}

fn run(rx: Receiver<AudioCmd>, ok: Arc<AtomicBool>) {
    // Keep the command forwarder responsive so keypress beeps reach the
    // real-time callback immediately without contending with background work.
    util::set_thread_priority(util::prio::PANEL);

    // Watch for OS default-device changes (e.g. speakers -> headphones) on a
    // separate low-priority thread. On Windows WASAPI, querying the default
    // MMDevice endpoint can block 40-150 ms; isolating it here guarantees
    // device polling NEVER stalls a KeyBeep on the hot forwarding path.
    let active_device: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let switch_flag = Arc::new(AtomicBool::new(false));
    let stop_watch = Arc::new(AtomicBool::new(false));
    {
        let active2 = active_device.clone();
        let switch2 = switch_flag.clone();
        let stop2 = stop_watch.clone();
        let _ = std::thread::Builder::new().name("audio-watch".into()).spawn(move || {
            util::set_thread_priority(util::prio::BACKGROUND);
            while !stop2.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1200));
                if stop2.load(Ordering::Relaxed) {
                    break;
                }
                let cur = active2.lock().ok().and_then(|g| g.clone());
                if let Some(using) = cur {
                    if let Some(now_default) = default_output_name() {
                        if using != now_default {
                            util::log(&format!("sound output switched to {}", now_default));
                            switch2.store(true, Ordering::Release);
                        }
                    }
                }
            }
        });
    }

    let mut state = State::default();
    let mut logged = false;
    // Short burst commands asked for while the output was rebuilding (Windows
    // resets it when a device changes or wakes up). Replayed if still fresh.
    let mut pending: Vec<(AudioCmd, Instant)> = Vec::new();
    let mut backoff = Duration::from_millis(100);
    loop {
        switch_flag.store(false, Ordering::Relaxed);
        let failed = Arc::new(AtomicBool::new(false));
        let (itx, irx) = mpsc::channel::<AudioCmd>();
        let built = build_stream(irx, failed.clone());
        // Which output this stream plays into, to notice a switch.
        let using = built.as_ref().ok().and_then(|(_, name)| name.clone());
        if let Ok(mut g) = active_device.lock() {
            *g = using.clone();
        }
        let stream = built.map(|(s, _)| s);
        match &stream {
            Ok(_) => {
                ok.store(true, Ordering::Relaxed);
                if logged {
                    util::log("audio output is back");
                }
                logged = false;
                backoff = Duration::from_millis(100);
                for c in state.replay() {
                    let _ = itx.send(c);
                }
                for (c, at) in pending.drain(..) {
                    if at.elapsed() < Duration::from_millis(700) {
                        let _ = itx.send(c);
                    }
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
        let mut switched = false;
        loop {
            match rx.recv_timeout(Duration::from_millis(150)) {
                Ok(cmd) => {
                    state.remember(&cmd);
                    let alive = stream.is_ok() && !failed.load(Ordering::Relaxed);
                    if alive {
                        let _ = itx.send(cmd);
                    } else if State::worth_replaying(&cmd) {
                        if pending.len() >= 4 {
                            pending.remove(0);
                        }
                        pending.push((cmd, Instant::now()));
                    }
                    // Drain any commands queued back-to-back in the same turn
                    // (e.g. KeyBeep + Tone + Speak) without looping timeout.
                    while let Ok(more) = rx.try_recv() {
                        state.remember(&more);
                        if alive {
                            let _ = itx.send(more);
                        } else if State::worth_replaying(&more) {
                            if pending.len() >= 4 {
                                pending.remove(0);
                            }
                            pending.push((more, Instant::now()));
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    stop_watch.store(true, Ordering::Relaxed);
                    return;
                }
            }
            if stream.is_err() || failed.load(Ordering::Relaxed) {
                break;
            }
            if switch_flag.swap(false, Ordering::Acquire) {
                switched = true;
                break;
            }
        }
        if let Ok(mut g) = active_device.lock() {
            *g = None;
        }
        drop(stream);
        ok.store(false, Ordering::Relaxed);
        // Reconnect fast. A device that's just waking up is usually back in a
        // moment, and a waiting beep gets the quickest retry of all.
        let wait = if switched || !pending.is_empty() { Duration::from_millis(50) } else { backoff };
        std::thread::sleep(wait);
        backoff = if switched { Duration::from_millis(100) } else { (backoff * 2).min(Duration::from_millis(1500)) };
    }
}

/// The output the system is set to use right now.
fn default_output_name() -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    cpal::default_host().default_output_device().and_then(|d| d.name().ok())
}

/// Fills one buffer from the synth. A bug in the synth must never take the
/// audio thread down with it, so a panic just gives a buffer of silence.
fn render<T: Copy>(synth: &mut synth::Synth, data: &mut [T], channels: usize, silence: T, conv: impl Fn(f32) -> T) {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        for frame in data.chunks_mut(channels) {
            let s = conv(synth.next());
            for o in frame.iter_mut() {
                *o = s;
            }
        }
    }));
    if res.is_err() {
        for o in data.iter_mut() {
            *o = silence;
        }
    }
}

/// Opens the system's default output. Also returns its name, if it has one.
fn build_stream(rx: Receiver<AudioCmd>, failed: Arc<AtomicBool>) -> Result<(cpal::Stream, Option<String>), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no audio output device".to_string())?;
    let name = device.name().ok();
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels.max(1) as usize;
    let rate = config.sample_rate.0 as f32;
    let mut synth = synth::Synth::new(rate);
    // The sound thread gets a high priority the first time it runs, so heavy
    // background work can never make the keypad stutter or beep late.
    let mut boosted = false;
    util::log(&format!(
        "audio output: {}, {} Hz, {} channels, {:?}",
        name.as_deref().unwrap_or("unnamed device"),
        config.sample_rate.0,
        channels,
        format
    ));
    let flag = failed.clone();
    let on_error = move |e: cpal::StreamError| {
        util::log(&format!("audio stream error: {}", e));
        flag.store(true, Ordering::Relaxed);
    };
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                if !boosted {
                    boosted = true;
                    util::set_thread_priority(util::prio::AUDIO);
                }
                while let Ok(c) = rx.try_recv() {
                    synth.apply(c);
                }
                render(&mut synth, data, channels, 0.0f32, |s| s);
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                if !boosted {
                    boosted = true;
                    util::set_thread_priority(util::prio::AUDIO);
                }
                while let Ok(c) = rx.try_recv() {
                    synth.apply(c);
                }
                render(&mut synth, data, channels, 0i16, |s| (s.clamp(-1.0, 1.0) * 32767.0) as i16);
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I32 => device.build_output_stream(
            &config,
            move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                if !boosted {
                    boosted = true;
                    util::set_thread_priority(util::prio::AUDIO);
                }
                while let Ok(c) = rx.try_recv() {
                    synth.apply(c);
                }
                render(&mut synth, data, channels, 0i32, |s| (s.clamp(-1.0, 1.0) as f64 * 2_147_483_647.0) as i32);
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config,
            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                if !boosted {
                    boosted = true;
                    util::set_thread_priority(util::prio::AUDIO);
                }
                while let Ok(c) = rx.try_recv() {
                    synth.apply(c);
                }
                render(&mut synth, data, channels, 32768u16, |s| ((s.clamp(-1.0, 1.0) * 0.5 + 0.5) * 65535.0) as u16);
            },
            on_error,
            None,
        ),
        other => return Err(format!("unsupported sample format {:?}", other)),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, name))
}
