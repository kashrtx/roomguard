//! Sound. Every beep, chime and siren is synthesized live from a physical
//! model of the real device; nothing is a recording. The audio thread keeps
//! the current state (sounder pattern, bell, siren choice, volumes) so it can
//! rebuild the output stream if the device changes or disappears, which also
//! tells the panel when the "bell circuit" is broken.

pub mod memo;
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
    PlayMemo(memo::Clip),
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
    /// A run of pitched beeps (hz, on, off) after a pause: the 6160's low
    /// "tu-tu-tu" when it arms STAY, NIGHT-STAY or INSTANT.
    ToneSeq { delay_ms: u16, tones: Vec<(u16, u16, u16)> },
    Silence,
    /// The pointer is near the keypad: make sure the output is awake.
    Wake,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Route {
    Keypad,
    Siren,
    Both,
}

impl Route {
    fn accepts(self, cmd: &AudioCmd) -> bool {
        match cmd {
            AudioCmd::Volumes { .. } | AudioCmd::Silence => true,
            AudioCmd::Bell(_) | AudioCmd::Squawk(_) | AudioCmd::Siren { .. } | AudioCmd::Preview { .. } => self != Self::Keypad,
            _ => self != Self::Siren,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Output {
    route: Route,
    device: Option<String>,
}

enum WorkerCmd {
    Audio(AudioCmd),
    Output(Option<Output>),
}

#[derive(Clone)]
struct OutputWorker {
    tx: Sender<WorkerCmd>,
    ok: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct AudioHandle {
    keypad: OutputWorker,
    siren: OutputWorker,
    devices: Arc<Mutex<Vec<String>>>,
    split: Arc<AtomicBool>,
}

impl AudioHandle {
    pub fn send(&self, cmd: AudioCmd) {
        // Keep the siren worker's persistent state current even while it is idle.
        let _ = self.siren.tx.send(WorkerCmd::Audio(cmd.clone()));
        let _ = self.keypad.tx.send(WorkerCmd::Audio(cmd));
    }

    pub fn configure(&self, keypad: Option<String>, siren: Option<String>) {
        let separate = siren.is_some() && siren != keypad;
        self.split.store(separate, Ordering::Relaxed);
        let keypad_output = Output { route: if separate { Route::Keypad } else { Route::Both }, device: keypad };
        let siren_output = separate.then_some(Output { route: Route::Siren, device: siren });
        let _ = self.keypad.tx.send(WorkerCmd::Output(Some(keypad_output)));
        let _ = self.siren.tx.send(WorkerCmd::Output(siren_output));
    }

    pub fn devices(&self) -> Vec<String> {
        self.devices.lock().map(|devices| devices.clone()).unwrap_or_default()
    }

    pub fn keypad_ok(&self) -> bool {
        self.keypad.ok.load(Ordering::Relaxed)
    }

    /// The panel's bell circuit follows the selected siren output.
    pub fn is_ok(&self) -> bool {
        if self.split.load(Ordering::Relaxed) {
            self.siren.ok.load(Ordering::Relaxed)
        } else {
            self.keypad_ok()
        }
    }
}

pub fn start() -> AudioHandle {
    let devices = Arc::new(Mutex::new(Vec::new()));
    AudioHandle {
        keypad: start_output("audio-keypad", Some(devices.clone())),
        siren: start_output("audio-siren", None),
        devices,
        split: Arc::new(AtomicBool::new(false)),
    }
}

fn start_output(name: &'static str, devices: Option<Arc<Mutex<Vec<String>>>>) -> OutputWorker {
    let (tx, rx) = mpsc::channel();
    let ok = Arc::new(AtomicBool::new(false));
    let running = ok.clone();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || run(rx, running, devices));
    if let Err(e) = spawned {
        util::log(&format!("could not start {}: {}", name, e));
    }
    OutputWorker { tx, ok }
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
        matches!(c, AudioCmd::KeyBeep | AudioCmd::Beeps { .. } | AudioCmd::Chime(_) | AudioCmd::Tone { .. } | AudioCmd::ToneSeq { .. } | AudioCmd::Speak(_))
    }

    fn replay(&self) -> Vec<AudioCmd> {
        [&self.voice, &self.buzzer_level, &self.voice_level, &self.volumes, &self.siren, &self.sounder, &self.bell, &self.ring]
            .iter()
            .filter_map(|c| (*c).clone())
            .collect()
    }
}

fn run(rx: Receiver<WorkerCmd>, ok: Arc<AtomicBool>, devices: Option<Arc<Mutex<Vec<String>>>>) {
    util::set_thread_priority(util::prio::PANEL);

    // Endpoint enumeration can block on WASAPI. Keep it off the command path.
    let active_default = Arc::new(Mutex::new(None::<String>));
    let switch_flag = Arc::new(AtomicBool::new(false));
    let stop_watch = Arc::new(AtomicBool::new(false));
    if let Some(devices) = devices {
        let active = active_default.clone();
        let switched = switch_flag.clone();
        let stop = stop_watch.clone();
        let _ = std::thread::Builder::new().name("audio-watch".into()).spawn(move || {
            util::set_thread_priority(util::prio::BACKGROUND);
            while !stop.load(Ordering::Relaxed) {
                let names = output_names();
                if let Ok(mut list) = devices.lock() {
                    *list = names;
                }
                let current = active.lock().ok().and_then(|name| name.clone());
                if let Some(current) = current {
                    if default_output_name().is_some_and(|name| name != current) {
                        switched.store(true, Ordering::Release);
                    }
                }
                std::thread::sleep(Duration::from_millis(1200));
            }
        });
    }

    let mut output: Option<Output> = None;
    let mut state = State::default();
    let mut logged = false;
    let mut pending = Vec::new();
    let mut backoff = Duration::from_millis(100);
    loop {
        let Some(current) = &output else {
            match rx.recv() {
                Ok(WorkerCmd::Output(next)) => output = next,
                Ok(WorkerCmd::Audio(cmd)) => state.remember(&cmd),
                Err(_) => break,
            }
            continue;
        };
        let current = current.clone();
        switch_flag.store(false, Ordering::Relaxed);
        let failed = Arc::new(AtomicBool::new(false));
        let (tx, input) = mpsc::channel();
        // Prime the callback before opening the device, including an alarm
        // already sounding when its speaker comes back.
        for cmd in state.replay().into_iter().filter(|cmd| current.route.accepts(cmd)) {
            let _ = tx.send(cmd);
        }
        let built = build_stream(input, failed.clone(), current.device.as_deref());
        if let Ok(mut name) = active_default.lock() {
            *name = if current.device.is_none() { built.as_ref().ok().and_then(|(_, name)| name.clone()) } else { None };
        }
        let stream = built.map(|(stream, _)| stream);
        match &stream {
            Ok(_) => {
                ok.store(true, Ordering::Relaxed);
                if logged {
                    util::log(&format!("{:?} audio output is back", current.route));
                }
                logged = false;
                backoff = Duration::from_millis(100);
                for (cmd, at) in pending.drain(..) {
                    if Instant::elapsed(&at) < Duration::from_millis(700) && current.route.accepts(&cmd) {
                        let _ = tx.send(cmd);
                    }
                }
            }
            Err(e) => {
                if !logged {
                    util::log(&format!("{:?} audio output unavailable: {}", current.route, e));
                    logged = true;
                }
            }
        }

        let deadline = Instant::now() + backoff;
        let retry = loop {
            let alive = stream.is_ok() && !failed.load(Ordering::Relaxed);
            if switch_flag.swap(false, Ordering::Acquire) || (stream.is_ok() && !alive) {
                break true;
            }
            let wait = if alive { Duration::from_millis(150) } else { deadline.saturating_duration_since(Instant::now()) };
            if wait.is_zero() {
                backoff = (backoff * 2).min(Duration::from_millis(1500));
                break true;
            }
            match rx.recv_timeout(wait) {
                Ok(WorkerCmd::Output(next)) => {
                    if next != output {
                        output = next;
                        backoff = Duration::from_millis(100);
                        pending.clear();
                        break true;
                    }
                }
                Ok(WorkerCmd::Audio(cmd)) => {
                    state.remember(&cmd);
                    if matches!(cmd, AudioCmd::Silence) {
                        pending.clear();
                    }
                    if current.route.accepts(&cmd) {
                        if alive {
                            let _ = tx.send(cmd);
                        } else if State::worth_replaying(&cmd) {
                            if pending.len() >= 4 {
                                pending.remove(0);
                            }
                            pending.push((cmd, Instant::now()));
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break false,
            }
        };
        if let Ok(mut name) = active_default.lock() {
            *name = None;
        }
        ok.store(false, Ordering::Relaxed);
        drop(stream);
        if !retry {
            break;
        }
    }
    stop_watch.store(true, Ordering::Relaxed);
}

fn output_names() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    let mut names: Vec<_> = cpal::default_host()
        .output_devices()
        .into_iter()
        .flatten()
        .filter_map(|device| device.name().ok())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
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

/// Opens the selected output, or the system default when none is selected.
fn build_stream(rx: Receiver<AudioCmd>, failed: Arc<AtomicBool>, selected: Option<&str>) -> Result<(cpal::Stream, Option<String>), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = match selected {
        Some(name) => host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|device| device.name().ok().as_deref() == Some(name))
            .ok_or_else(|| format!("{} is disconnected", name))?,
        None => host.default_output_device().ok_or_else(|| "no audio output device".to_string())?,
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separate_outputs_keep_keypad_and_siren_commands_apart() {
        for cmd in [AudioCmd::KeyBeep, AudioCmd::Speak(vec!["ready".into()]), AudioCmd::Ring(true)] {
            assert!(Route::Keypad.accepts(&cmd));
            assert!(!Route::Siren.accepts(&cmd));
            assert!(Route::Both.accepts(&cmd));
        }
        for cmd in [
            AudioCmd::Bell(BellPattern::Steady),
            AudioCmd::Squawk(1),
            AudioCmd::Preview { model: SirenModel::Wave2F, warble: true, ms: 1000 },
        ] {
            assert!(!Route::Keypad.accepts(&cmd));
            assert!(Route::Siren.accepts(&cmd));
            assert!(Route::Both.accepts(&cmd));
        }
        assert!(Route::Keypad.accepts(&AudioCmd::Silence));
        assert!(Route::Siren.accepts(&AudioCmd::Silence));
    }
}
