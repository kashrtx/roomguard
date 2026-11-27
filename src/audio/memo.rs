//! Vista voice memos and the Sound settings microphone test.

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, Sample, SizedSample};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

pub const MAX_SECONDS: u64 = 150;

#[derive(Clone, Debug)]
pub struct Clip {
    pub samples: Arc<[f32]>,
    pub rate: u32,
}

pub enum Command {
    Device(Option<String>),
    Record(Instant),
    Stop(Instant),
    Test(Option<String>),
    Cancel,
}

pub type Result = std::result::Result<Clip, String>;

pub enum Event {
    Memo { id: Instant, result: Result },
    Testing(String),
    Test(Result),
}

enum Recording {
    Memo(Instant, Capture),
    Test(Instant, Capture),
}

pub fn start(deliver: impl Fn(Event) + Send + Sync + 'static) -> mpsc::Sender<Command> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("keypad-memo".into()).spawn(move || {
        let mut active = None;
        let mut selected = None;
        loop {
            let command = if let Some(Recording::Test(until, _)) = &active {
                rx.recv_timeout(until.saturating_duration_since(Instant::now()))
            } else {
                rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            };
            match command {
                Ok(Command::Device(device)) => selected = device,
                Ok(Command::Record(id)) => {
                    if matches!(active.take(), Some(Recording::Test(_, _))) {
                        deliver(Event::Test(Err("Microphone test was interrupted by a keypad recording".into())));
                    }
                    match Capture::open(selected.as_deref(), MAX_SECONDS) {
                        Ok(capture) => active = Some(Recording::Memo(id, capture)),
                        Err(error) => deliver(Event::Memo { id, result: Err(error) }),
                    }
                }
                Ok(Command::Stop(id)) => {
                    if matches!(active.as_ref(), Some(Recording::Memo(recorded, _)) if *recorded == id) {
                        if let Some(Recording::Memo(_, capture)) = active.take() {
                            deliver(Event::Memo { id, result: capture.finish() });
                        }
                    }
                }
                Ok(Command::Test(device)) => {
                    if matches!(active, Some(Recording::Memo(..))) {
                        deliver(Event::Test(Err("Stop the keypad recording before testing the microphone".into())));
                        continue;
                    }
                    drop(active.take());
                    match Capture::open(device.as_deref(), 3) {
                        Ok(capture) => {
                            deliver(Event::Testing(format!("Recording from {} for 3 seconds. Speak now.", capture.label)));
                            active = Some(Recording::Test(Instant::now() + Duration::from_secs(3), capture));
                        }
                        Err(error) => deliver(Event::Test(Err(error))),
                    }
                }
                Ok(Command::Cancel) => {
                    if matches!(active.take(), Some(Recording::Test(_, _))) {
                        deliver(Event::Test(Err("Microphone test canceled".into())));
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(Recording::Test(_, capture)) = active.take() {
                        deliver(Event::Test(capture.finish()));
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    if let Err(error) = spawned {
        crate::util::log(&format!("could not start the memo recorder: {}", error));
    }
    tx
}

struct Capture {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    stream_error: Arc<Mutex<Option<String>>>,
    rate: u32,
    label: String,
}

impl Capture {
    fn open(selected: Option<&str>, seconds: u64) -> std::result::Result<Self, String> {
        let device = super::choose_device(true, selected)?;
        let label = super::device_label(&device);
        let supported = device.default_input_config().map_err(|e| format!("{label}: {e}"))?;
        let config = supported.config();
        let stride = (config.sample_rate / 8000).max(1);
        let rate = config.sample_rate / stride;
        let limit = rate as usize * seconds as usize;
        let samples = Arc::new(Mutex::new(Vec::with_capacity(limit)));
        let stream_error = Arc::new(Mutex::new(None));
        let error_slot = stream_error.clone();
        let on_error = move |error: cpal::StreamError| {
            if let Ok(mut slot) = error_slot.lock() {
                *slot = Some(error.to_string());
            }
        };
        macro_rules! open {
            ($ty:ty) => { input_stream::<$ty>(&device, &config, samples.clone(), stride, limit, on_error) };
        }
        let stream = pcm!(supported.sample_format(), open).map_err(|e| format!("{label}: {e}"))?;
        stream.play().map_err(|e| format!("{label}: {e}"))?;
        crate::util::log(&format!("memo input: {label}, {} Hz, {} channels, {}", config.sample_rate, config.channels, supported.sample_format()));
        Ok(Self { stream, samples, stream_error, rate, label })
    }

    fn finish(self) -> Result {
        drop(self.stream);
        if let Ok(mut slot) = self.stream_error.lock() {
            if let Some(error) = slot.take() {
                return Err(format!("{}: {}", self.label, error));
            }
        }
        let samples = std::mem::take(&mut *self.samples.lock().map_err(|_| "Microphone buffer unavailable")?);
        validate_signal(&samples)?;
        Ok(Clip { samples: samples.into(), rate: self.rate })
    }
}

fn validate_signal(samples: &[f32]) -> std::result::Result<(), String> {
    if samples.is_empty() {
        return Err("The microphone supplied no audio. Check input selection and microphone permission.".into());
    }
    if samples.iter().all(|sample| sample.abs() < 0.0005) {
        return Err("The microphone supplied silence. Check its mute switch and input level.".into());
    }
    Ok(())
}

fn input_stream<T: SizedSample>(
    device: &cpal::Device, config: &cpal::StreamConfig, samples: Arc<Mutex<Vec<f32>>>,
    stride: u32, limit: usize, on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> std::result::Result<cpal::Stream, cpal::BuildStreamError>
where
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let mut sum = 0.0;
    let mut count = 0;
    device.build_input_stream(config, move |data: &[T], _| {
        let Ok(mut samples) = samples.try_lock() else { return };
        if samples.len() == limit {
            return;
        }
        // Some multi-channel USB interfaces put the only connected
        // microphone on a single channel. Avoid averaging it with silence
        // or canceling an out-of-phase pair.
        let channel = (0..channels).max_by(|a, b| {
            let peak = |channel| data.chunks_exact(channels)
                .map(|frame| f32::from_sample(frame[channel]).abs()).fold(0.0, f32::max);
            peak(*a).total_cmp(&peak(*b))
        }).unwrap_or(0);
        for frame in data.chunks_exact(channels) {
            let sample = f32::from_sample(frame[channel]);
            sum += if sample.is_finite() { sample } else { 0.0 };
            count += 1;
            if count == stride {
                samples.push(sum / stride as f32);
                sum = 0.0;
                count = 0;
                if samples.len() == limit {
                    break;
                }
            }
        }
    }, on_error, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muted_and_missing_mics_get_distinct_errors() {
        assert!(validate_signal(&[]).unwrap_err().contains("no audio"));
        assert!(validate_signal(&[0.0; 8]).unwrap_err().contains("silence"));
        assert!(validate_signal(&[0.0, -0.02, 0.01]).is_ok());
    }
}
