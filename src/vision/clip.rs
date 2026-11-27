//! A small, in-memory camera buffer for opt-in phone alert GIFs.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::frame::Rgb;

const PRE_ROLL: Duration = Duration::from_millis(1500);
pub(super) const POST_ROLL: Duration = Duration::from_millis(1500);
const INTERVAL: Duration = Duration::from_millis(125);
const MAX_FRAMES: usize = 28;
// Leave room under the hosted ntfy.sh limit of 2 MB.
const MAX_BYTES: usize = 1_950_000;

#[derive(Default)]
pub(super) struct Buffer {
    frames: VecDeque<(Instant, Rgb)>,
}

impl Buffer {
    pub(super) fn clear(&mut self) {
        self.frames.clear();
    }

    pub(super) fn sample(&mut self, now: Instant, rgb: &Rgb) {
        if rgb.w == 0 || rgb.h == 0 {
            return;
        }
        if self.frames.back().is_some_and(|(at, _)| now.saturating_duration_since(*at) < INTERVAL) {
            return;
        }

        let scale = (320.0 / rgb.w as f32).min(180.0 / rgb.h as f32).min(1.0);
        let w = ((rgb.w as f32 * scale).round() as usize).max(1);
        let h = ((rgb.h as f32 * scale).round() as usize).max(1);
        if self.frames.back().is_some_and(|(_, last)| last.w != w || last.h != h) {
            self.clear();
        }
        while self.frames.front().is_some_and(|(at, _)| now.saturating_duration_since(*at) > PRE_ROLL + POST_ROLL + INTERVAL)
            || self.frames.len() >= MAX_FRAMES
        {
            self.frames.pop_front();
        }

        let mut data = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            let row = y * rgb.h / h * rgb.w;
            for x in 0..w {
                let index = (row + x * rgb.w / w) * 3;
                data.extend_from_slice(&rgb.data[index..index + 3]);
            }
        }
        self.frames.push_back((now, Rgb { w, h, data }));
    }

    pub(super) fn encode(&self, event: Instant) -> Option<Vec<u8>> {
        let start = event.checked_sub(PRE_ROLL).unwrap_or(event);
        let end = event + POST_ROLL;
        let mut frames = self.frames.iter().filter(|(at, _)| *at >= start && *at <= end).peekable();
        let (_, first) = frames.peek()?;
        let (w, h) = (first.w as u16, first.h as u16);

        // One shared RGB palette keeps encoding fast enough for the camera
        // thread and avoids fitting a new palette for every sampled frame.
        let mut palette = Vec::with_capacity(256 * 3);
        for i in 0..=255u16 {
            palette.extend([
                ((i >> 5) * 255 / 7) as u8,
                (((i >> 2) & 7) * 255 / 7) as u8,
                ((i & 3) * 255 / 3) as u8,
            ]);
        }

        let mut bytes = Vec::new();
        let mut count = 0;
        {
            let mut encoder = gif::Encoder::new(&mut bytes, w, h, &palette).ok()?;
            encoder.set_repeat(gif::Repeat::Infinite).ok()?;
            while let Some((at, rgb)) = frames.next() {
                let delay = frames.peek().map(|(next, _)| next.saturating_duration_since(*at)).unwrap_or(INTERVAL);
                let indexed = rgb.data.chunks_exact(3).map(|p| (p[0] & 0xe0) | ((p[1] & 0xe0) >> 3) | (p[2] >> 6)).collect();
                encoder.write_frame(&gif::Frame {
                    width: w,
                    height: h,
                    delay: (delay.as_millis() / 10).clamp(1, 50) as u16,
                    buffer: Cow::Owned(indexed),
                    ..Default::default()
                }).ok()?;
                count += 1;
            }
        }
        (count >= 2 && bytes.len() <= MAX_BYTES).then_some(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_has_ordered_frames_from_both_sides_of_the_event() {
        let start = Instant::now();
        let mut buffer = Buffer::default();
        let event = start + Duration::from_secs(2);
        for index in 0..=29 {
            let rgb = Rgb { w: 12, h: 8, data: [index * 8, 0, 0].repeat(96) };
            buffer.sample(start + INTERVAL * u32::from(index), &rgb);
        }

        let bytes = buffer.encode(event).expect("motion GIF");
        let mut decoder = gif::DecodeOptions::new().read_info(bytes.as_slice()).unwrap();
        assert_eq!((decoder.width(), decoder.height()), (12, 8));
        let mut colors = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            colors.push(frame.buffer[0]);
            assert_eq!(frame.delay, 12);
        }
        assert_eq!(colors.len(), 25);
        assert_eq!(colors[0], 32);
        assert_eq!(colors[24], 224);
    }

    #[test]
    fn a_single_frame_or_frames_from_a_different_event_cannot_make_a_clip() {
        let now = Instant::now();
        let mut buffer = Buffer::default();
        let rgb = Rgb { w: 4, h: 3, data: vec![0; 36] };
        buffer.sample(now, &rgb);
        assert!(buffer.encode(now).is_none());
        buffer.sample(now + INTERVAL, &rgb);
        assert!(buffer.encode(now + Duration::from_secs(20)).is_none());
        buffer.clear();
        assert!(buffer.encode(now).is_none());
    }
}
