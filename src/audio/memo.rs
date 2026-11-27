//! The Vista keypad opens the microphone only while a user records a memo.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

pub const MAX_SECONDS: u64 = 150;

#[derive(Clone, Debug)]
pub struct Clip {
    pub samples: Arc<[f32]>,
    pub rate: u32,
}

pub enum Command {
    Record(Instant),
    Stop(Instant),
    Cancel,
}

pub type Result = std::result::Result<Clip, String>;

pub fn start(deliver: impl Fn(Instant, Result) + Send + Sync + 'static) -> mpsc::Sender<Command> {
    let (tx, rx) = mpsc::channel();
    let deliver = Arc::new(deliver);
    let spawned = std::thread::Builder::new().name("keypad-memo".into()).spawn(move || {
        let mut active = None;
        for command in rx {
            match command {
                Command::Record(id) => {
                    drop(active.take());
                    let report = deliver.clone();
                    match Capture::open(move |error| report(id, Err(error.to_string()))) {
                        Ok(capture) => active = Some((id, capture)),
                        Err(error) => deliver(id, Err(error)),
                    }
                }
                Command::Stop(id) => {
                    if active.as_ref().is_some_and(|(recorded, _)| *recorded == id) {
                        if let Some((_, capture)) = active.take() {
                            deliver(id, capture.finish());
                        }
                    }
                }
                Command::Cancel => active = None,
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
    rate: u32,
}

impl Capture {
    fn open(on_error: impl FnMut(cpal::StreamError) + Send + 'static) -> std::result::Result<Self, String> {
        let device = cpal::default_host().default_input_device().ok_or("No microphone is available")?;
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let config: cpal::StreamConfig = supported.clone().into();
        let channels = usize::from(config.channels.max(1));
        // Store one averaged sample per small group of frames to keep the
        // memo bounded even when the microphone runs at a high sample rate.
        let stride = (config.sample_rate.0 / 8000).max(1);
        let rate = config.sample_rate.0 / stride;
        let limit = rate as usize * MAX_SECONDS as usize;
        let samples = Arc::new(Mutex::new(Vec::with_capacity(limit)));
        let buffer = samples.clone();
        let process = move |frames: &mut dyn Iterator<Item = f32>| {
            if let Ok(mut samples) = buffer.try_lock() {
                let remaining = limit - samples.len();
                samples.extend(frames.take(remaining));
            }
        };

        macro_rules! input {
            ($ty:ty, $convert:expr) => {{
                let mut sum = 0.0;
                let mut count = 0;
                device.build_input_stream(
                    &config,
                    move |data: &[$ty], _| {
                        let mono = data.chunks_exact(channels).map(|frame| {
                            frame.iter().copied().map($convert).sum::<f32>() / channels as f32
                        });
                        process(&mut mono.filter_map(|sample| {
                            sum += sample;
                            count += 1;
                            if count == stride {
                                let average = sum / stride as f32;
                                sum = 0.0;
                                count = 0;
                                Some(average)
                            } else {
                                None
                            }
                        }));
                    },
                    on_error,
                    None,
                )
            }};
        }
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => input!(f32, |sample| sample),
            cpal::SampleFormat::I16 => input!(i16, |sample| sample as f32 / 32768.0),
            cpal::SampleFormat::U16 => input!(u16, |sample| sample as f32 / 32768.0 - 1.0),
            cpal::SampleFormat::I32 => input!(i32, |sample| sample as f32 / 2_147_483_648.0),
            format => return Err(format!("Unsupported microphone format: {:?}", format)),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self { stream, samples, rate })
    }

    fn finish(self) -> Result {
        drop(self.stream);
        let samples = std::mem::take(&mut *self.samples.lock().map_err(|_| "Microphone buffer unavailable")?);
        if samples.is_empty() {
            return Err("No audio was captured".to_string());
        }
        Ok(Clip { samples: samples.into(), rate: self.rate })
    }
}
