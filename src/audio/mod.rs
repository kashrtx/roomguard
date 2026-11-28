//! Sound. Every beep, chime and siren is synthesized live from a physical
//! model of the real device; nothing is a recording. The audio thread keeps
//! the current state (sounder pattern, bell, siren choice, volumes) so it can
//! rebuild the output stream if the device changes or disappears, which also
//! tells the panel when the "bell circuit" is broken.

// Dispatch to CPAL's typed PCM callbacks. Device mix formats can differ
// between endpoints on the same computer.
macro_rules! pcm {
    ($format:expr, $open:ident) => {
        match $format {
            cpal::SampleFormat::F32 => $open!(f32),
            cpal::SampleFormat::F64 => $open!(f64),
            cpal::SampleFormat::I8 => $open!(i8),
            cpal::SampleFormat::I16 => $open!(i16),
            cpal::SampleFormat::I24 => $open!(cpal::I24),
            cpal::SampleFormat::I32 => $open!(i32),
            cpal::SampleFormat::I64 => $open!(i64),
            cpal::SampleFormat::U8 => $open!(u8),
            cpal::SampleFormat::U16 => $open!(u16),
            cpal::SampleFormat::U24 => $open!(cpal::U24),
            cpal::SampleFormat::U32 => $open!(u32),
            cpal::SampleFormat::U64 => $open!(u64),
            other => return Err(format!("Unsupported audio format: {other}")),
        }
    };
}

pub mod memo;
pub mod synth;
pub mod voice;

use std::sync::atomic::{AtomicU32, AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

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
    PlayTest(memo::Clip),
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
    ClearSiren,
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
            // Settings, silencing, clearing the siren and the wake-up nudge
            // belong to every output, or a separate siren speaker misses them.
            AudioCmd::Volumes { .. } | AudioCmd::Silence | AudioCmd::ClearSiren | AudioCmd::Wake => true,
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
pub struct Endpoint {
    pub id: String,
    pub label: String,
    // The previous build stored the Windows friendly name.
    legacy: String,
}

#[derive(Clone, Default)]
pub struct Endpoints {
    pub outputs: Vec<Endpoint>,
    pub inputs: Vec<Endpoint>,
}

impl Endpoint {
    pub fn matches(&self, selection: Option<&str>) -> bool {
        selection.is_some_and(|value| value == self.id || value == self.legacy)
    }
}

#[derive(Clone)]
struct OutputWorker {
    tx: Sender<WorkerCmd>,
    ok: Arc<AtomicBool>,
    status: Arc<Mutex<String>>,
    device_id: Arc<Mutex<Option<String>>>,
    /// Recent peak level of what this output plays (f32 bits), for the meter.
    level: Arc<AtomicU32>,
}

#[derive(Clone)]
pub struct AudioHandle {
    keypad: OutputWorker,
    siren: OutputWorker,
    devices: Arc<Mutex<Endpoints>>,
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

    pub fn devices(&self) -> Endpoints {
        self.devices.lock().map(|devices| devices.clone()).unwrap_or_default()
    }

    pub fn test_keypad(&self) {
        const RATE: u32 = 8000;
        let samples: Arc<[f32]> = (0..RATE).map(|sample| {
            let t = sample as f32 / RATE as f32;
            let (frequency, elapsed) = if t < 0.5 { (880.0, t) } else { (1180.0, t - 0.5) };
            let envelope = (elapsed / 0.01).min(1.0) * ((0.35 - elapsed) / 0.02).clamp(0.0, 1.0);
            (t * frequency * std::f32::consts::TAU).sin() * envelope * 0.4
        }).collect();
        self.send(AudioCmd::PlayTest(memo::Clip { samples, rate: RATE }));
    }

    pub fn same_output(&self) -> bool {
        if !self.split.load(Ordering::Relaxed) || !self.keypad_ok() || !self.is_ok() {
            return false;
        }
        let keypad = self.keypad.device_id.lock().ok().and_then(|id| id.clone());
        let siren = self.siren.device_id.lock().ok().and_then(|id| id.clone());
        keypad.is_some() && keypad == siren
    }

    pub fn output_status(&self, siren: bool) -> String {
        let worker = if siren && self.split.load(Ordering::Relaxed) { &self.siren } else { &self.keypad };
        worker.status.lock().map(|status| status.clone()).unwrap_or_default()
    }

    /// How loud this output is right now (0 to 1), for the meter beside each
    /// output in Settings. It shows whether RoomGuard is sending sound to that
    /// speaker at all, which separates a RoomGuard problem from a Windows one.
    pub fn output_level(&self, siren: bool) -> f32 {
        let worker = if siren && self.split.load(Ordering::Relaxed) { &self.siren } else { &self.keypad };
        f32::from_bits(worker.level.load(Ordering::Relaxed))
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
    let devices = Arc::new(Mutex::new(Endpoints::default()));
    AudioHandle {
        keypad: start_output("audio-keypad", Some(devices.clone())),
        siren: start_output("audio-siren", None),
        devices,
        split: Arc::new(AtomicBool::new(false)),
    }
}

fn start_output(name: &'static str, devices: Option<Arc<Mutex<Endpoints>>>) -> OutputWorker {
    let (tx, rx) = mpsc::channel();
    let ok = Arc::new(AtomicBool::new(false));
    let running = ok.clone();
    let status = Arc::new(Mutex::new(String::new()));
    let status_rx = status.clone();
    let device_id = Arc::new(Mutex::new(None));
    let active_id = device_id.clone();
    let level = Arc::new(AtomicU32::new(0));
    let level_rx = level.clone();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || run(rx, running, status_rx, active_id, devices, level_rx));
    if let Err(e) = spawned {
        util::log(&format!("could not start {}: {}", name, e));
    }
    OutputWorker { tx, ok, status, device_id, level }
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
        matches!(c, AudioCmd::KeyBeep | AudioCmd::Beeps { .. } | AudioCmd::Chime(_) | AudioCmd::Tone { .. } | AudioCmd::ToneSeq { .. } | AudioCmd::Speak(_) | AudioCmd::Preview { .. } | AudioCmd::PlayMemo(_) | AudioCmd::PlayTest(_))
    }

    fn replay(&self) -> Vec<AudioCmd> {
        [&self.voice, &self.buzzer_level, &self.voice_level, &self.volumes, &self.siren, &self.sounder, &self.bell, &self.ring]
            .iter()
            .filter_map(|c| (*c).clone())
            .collect()
    }
}

fn run(
    rx: Receiver<WorkerCmd>,
    ok: Arc<AtomicBool>,
    status: Arc<Mutex<String>>,
    device_id: Arc<Mutex<Option<String>>>,
    devices: Option<Arc<Mutex<Endpoints>>>,
    level: Arc<AtomicU32>,
) {
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
                let host = cpal::default_host();
                let names = Endpoints {
                    outputs: list_endpoints(&host, false).into_iter().map(|(endpoint, _)| endpoint).collect(),
                    inputs: list_endpoints(&host, true).into_iter().map(|(endpoint, _)| endpoint).collect(),
                };
                if let Ok(mut list) = devices.lock() {
                    *list = names;
                }
                let current = active.lock().ok().and_then(|name| name.clone());
                if let Some(current) = current {
                    if default_output_id().is_some_and(|name| name != current) {
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
        let mut current = current.clone();
        switch_flag.store(false, Ordering::Relaxed);
        let failed = Arc::new(AtomicBool::new(false));
        let callbacks = Arc::new(AtomicU64::new(0));
        let (tx, input) = mpsc::channel();
        // Prime the callback before opening the device, including an alarm
        // already sounding when its speaker comes back.
        for cmd in state.replay().into_iter().filter(|cmd| current.route.accepts(cmd)) {
            let _ = tx.send(cmd);
        }
        let built = build_stream(input, failed.clone(), ok.clone(), callbacks.clone(), level.clone(), current.device.as_deref());
        if let Ok(mut id) = device_id.lock() {
            *id = built.as_ref().ok().and_then(|(_, id, _)| id.clone());
        }
        if let Ok(mut status) = status.lock() {
            *status = match &built {
                Ok((_, _, label)) => label.clone(),
                Err(error) => error.clone(),
            };
        }
        if let Ok(mut name) = active_default.lock() {
            *name = if current.device.is_none() { built.as_ref().ok().and_then(|(_, id, _)| id.clone()) } else { None };
        }
        let stream = built.map(|(stream, _, _)| stream);
        match &stream {
            Ok(_) => {
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
        let mut last_callback = Instant::now();
        let mut seen_callbacks = 0;
        let retry = loop {
            let alive = stream.is_ok() && !failed.load(Ordering::Relaxed);
            let count = callbacks.load(Ordering::Relaxed);
            if count != seen_callbacks {
                last_callback = Instant::now();
                seen_callbacks = count;
            }
            if alive && last_callback.elapsed() > Duration::from_secs(3) {
                if let Ok(mut status) = status.lock() {
                    *status = "Selected device opened but did not request audio".into();
                }
                break true;
            }
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
                        if let Some(next) = next.as_ref().filter(|next| alive && next.device == current.device) {
                            // Changing the siren route leaves the keypad stream
                            // and any keypad audio already playing intact.
                            if next.route == Route::Keypad {
                                let _ = tx.send(AudioCmd::ClearSiren);
                            }
                            for cmd in state.replay().into_iter().filter(|cmd| next.route.accepts(cmd) && !current.route.accepts(cmd)) {
                                let _ = tx.send(cmd);
                            }
                            current.route = next.route;
                            output = Some(next.clone());
                            continue;
                        }
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
        drop(stream);
        ok.store(false, Ordering::Relaxed);
        level.store(0, Ordering::Relaxed);
        if !retry {
            break;
        }
    }
    stop_watch.store(true, Ordering::Relaxed);
}

fn list_endpoints(host: &cpal::Host, input: bool) -> Vec<(Endpoint, cpal::Device)> {
    let devices: Vec<_> = if input {
        host.input_devices().map(|devices| devices.collect()).unwrap_or_default()
    } else {
        host.output_devices().map(|devices| devices.collect()).unwrap_or_default()
    };
    let mut result: Vec<_> = devices.into_iter().filter_map(|device| {
        let id = device.id().ok()?.to_string();
        let label = device_label(&device);
        Some((Endpoint { id, label: label.clone(), legacy: label }, device))
    }).collect();
    result.sort_unstable_by(|a, b| (&a.0.legacy, &a.0.id).cmp(&(&b.0.legacy, &b.0.id)));
    let mut first = 0;
    while first < result.len() {
        let end = first + result[first..].iter().take_while(|(endpoint, _)| endpoint.legacy == result[first].0.legacy).count();
        if end - first > 1 {
            for (index, (endpoint, _)) in result[first..end].iter_mut().enumerate() {
                endpoint.label = format!("{} ({})", endpoint.legacy, index + 1);
            }
        }
        first = end;
    }
    result
}

fn device_label(device: &cpal::Device) -> String {
    let description = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| device.description())).ok().and_then(Result::ok);
    description.map(|description| {
        // WASAPI exposes the Windows Sound settings name in the first
        // extended field when it differs from the short device name.
        if cfg!(windows) {
            description.extended().first().cloned().unwrap_or_else(|| description.name().to_string())
        } else {
            description.name().into()
        }
    }).unwrap_or_else(|| "Audio device".into())
}

fn choose_device(input: bool, selected: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    let Some(selected) = selected else {
        return if input { host.default_input_device() } else { host.default_output_device() }
            .ok_or_else(|| if input { "No microphone is available" } else { "No speaker is available" }.into());
    };
    let mut matching = list_endpoints(&host, input).into_iter()
        .filter(|(endpoint, _)| endpoint.matches(Some(selected)));
    let (_, device) = matching.next().ok_or("Selected audio device is unavailable")?;
    if matching.next().is_some() {
        return Err("Several devices share this name. Select the numbered device again.".into());
    }
    Ok(device)
}

fn default_output_id() -> Option<String> {
    cpal::default_host().default_output_device()?.id().ok().map(|id| id.to_string())
}

fn output_stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device, config: &cpal::StreamConfig, rx: Receiver<AudioCmd>,
    failed: Arc<AtomicBool>, ready: Arc<AtomicBool>, callbacks: Arc<AtomicU64>, level: Arc<AtomicU32>,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = usize::from(config.channels.max(1));
    let rate = config.sample_rate as f32;
    let mut synth = synth::Synth::new(rate);
    let mut boosted = false;
    let mut recovered = false;
    let stream_error = failed.clone();
    device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            if !boosted {
                boosted = true;
                util::set_thread_priority(util::prio::AUDIO);
            }
            // Commands are applied inside the guard too. A command that made
            // the synth panic used to take this output down for good: the
            // stream was rebuilt, the same lasting state replayed, and it
            // panicked again, so a separate siren speaker went silent forever.
            let render = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                while let Ok(cmd) = rx.try_recv() {
                    synth.apply(cmd);
                }
                let mut peak = 0.0f32;
                for frame in data.chunks_mut(channels) {
                    let sample = synth.next().clamp(-0.99999, 0.99999);
                    peak = peak.max(sample.abs());
                    frame.fill(T::from_sample(sample));
                }
                peak
            }));
            match render {
                Ok(peak) => {
                    // A peak meter that falls back over about a third of a second.
                    let held = f32::from_bits(level.load(Ordering::Relaxed)) * 0.92;
                    level.store(peak.max(held).to_bits(), Ordering::Relaxed);
                    callbacks.fetch_add(1, Ordering::Relaxed);
                    ready.store(true, Ordering::Relaxed);
                }
                Err(_) => {
                    // Start this output's synth over and keep the stream going.
                    data.fill(T::EQUILIBRIUM);
                    synth = synth::Synth::new(rate);
                    if !recovered {
                        recovered = true;
                        util::log("an audio output recovered from an internal error and was reset");
                    }
                    callbacks.fetch_add(1, Ordering::Relaxed);
                }
            }
        },
        move |error| {
            stream_error.store(true, Ordering::Relaxed);
            util::log(&format!("audio stream error: {error}"));
        },
        None,
    )
}

fn build_stream(
    rx: Receiver<AudioCmd>, failed: Arc<AtomicBool>, ready: Arc<AtomicBool>, callbacks: Arc<AtomicU64>, level: Arc<AtomicU32>,
    selected: Option<&str>,
) -> Result<(cpal::Stream, Option<String>, String), String> {
    let device = choose_device(false, selected)?;
    let id = device.id().ok().map(|id| id.to_string());
    let label = device_label(&device);
    let supported = device.default_output_config().map_err(|e| format!("{label}: {e}"))?;
    let format = supported.sample_format();
    let config = supported.config();
    macro_rules! open {
        ($ty:ty) => { output_stream::<$ty>(&device, &config, rx, failed, ready, callbacks, level) };
    }
    let stream = pcm!(format, open).map_err(|e| format!("{label}: {e}"))?;
    stream.play().map_err(|e| format!("{label}: {e}"))?;
    util::log(&format!("audio output: {label}, {} Hz, {} channels, {format}", config.sample_rate, config.channels));
    Ok((stream, id, label))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clearing_and_waking_reach_every_output() {
        for route in [Route::Keypad, Route::Siren, Route::Both] {
            assert!(route.accepts(&AudioCmd::ClearSiren), "{:?} gets ClearSiren", route);
            assert!(route.accepts(&AudioCmd::Wake), "{:?} gets Wake", route);
        }
    }

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
