//! Motion detection on a small gray image.
//!
//! A running background model learns the room. Pixels that differ from it by
//! more than the noise level are motion. Speckle is cleaned up, sudden whole
//! frame changes (lights, auto exposure) relearn instead of alarming, and the
//! background keeps learning slowly underneath movement so a person who stops
//! still is remembered for a while instead of vanishing instantly.

use crate::config::NRect;

pub const MW: usize = 160;

#[derive(Clone, Copy, Debug, Default)]
pub struct MotionOut {
    pub door_frac: f32,
    pub room_frac: f32,
    pub total_frac: f32,
    pub light_change: bool,
    pub mean: f32,
    pub std: f32,
    /// False while the model is warming up or the camera is moving.
    pub valid: bool,
}

pub struct Motion {
    pub w: usize,
    pub h: usize,
    bg: Vec<f32>,
    /// Per-pixel natural noise floor from empty-room calibration (0.0 when uncalibrated).
    bg_std: Vec<f32>,
    blur: Vec<u8>,
    raw: Vec<u8>,
    pub mask: Vec<u8>,
    ignore: Vec<u8>,
    ignore_key: Vec<NRect>,
    noise: f32,
    ready: bool,
    settle: u32,
}

impl Motion {
    pub fn new() -> Self {
        Motion {
            w: 0,
            h: 0,
            bg: Vec::new(),
            bg_std: Vec::new(),
            blur: Vec::new(),
            raw: Vec::new(),
            mask: Vec::new(),
            ignore: Vec::new(),
            ignore_key: Vec::new(),
            noise: 2.0,
            ready: false,
            settle: 0,
        }
    }

    pub fn reset(&mut self) {
        self.ready = false;
    }

    /// Seeds the background model and per-pixel noise floor from a calibrated
    /// empty-room snapshot, eliminating ghost blobs from furniture/bedding/monitors.
    pub fn apply_baseline(&mut self, mean: &[u8], std: &[u8], w: usize, h: usize) {
        let n = w * h;
        if n == 0 || mean.len() < n || std.len() < n {
            return;
        }
        self.w = w;
        self.h = h;
        self.blur = vec![0; n];
        self.raw = vec![0; n];
        self.mask = vec![0; n];
        self.bg = mean[..n].iter().map(|&v| v as f32).collect();
        self.bg_std = std[..n].iter().map(|&v| v as f32).collect();
        self.ignore = vec![0; n];
        self.ignore_key = vec![NRect::new(-1.0, -1.0, 0.0, 0.0)];
        self.ready = true;
        self.settle = 0;
    }

    /// Skip and relearn for a few frames, used while a PTZ camera turns.
    pub fn settle(&mut self, frames: u32) {
        self.settle = self.settle.max(frames);
    }

    pub fn process(&mut self, gray: &[u8], w: usize, h: usize, sensitivity: u8, door: Option<NRect>, ignore: &[NRect]) -> MotionOut {
        let n = w * h;
        if n == 0 || gray.len() < n {
            return MotionOut::default();
        }
        if !self.ready || w != self.w || h != self.h {
            self.w = w;
            self.h = h;
            self.blur = vec![0; n];
            self.raw = vec![0; n];
            self.mask = vec![0; n];
            box_blur(gray, w, h, &mut self.blur);
            self.bg = self.blur.iter().map(|&v| v as f32).collect();
            self.bg_std = vec![0.0; n];
            self.ignore = vec![0; n];
            self.ignore_key = vec![NRect::new(-1.0, -1.0, 0.0, 0.0)];
            self.ready = true;
            return MotionOut::default();
        }
        if self.ignore_key.as_slice() != ignore {
            for y in 0..h {
                let fy = (y as f32 + 0.5) / h as f32;
                for x in 0..w {
                    let fx = (x as f32 + 0.5) / w as f32;
                    self.ignore[y * w + x] = ignore.iter().any(|r| r.contains(fx, fy)) as u8;
                }
            }
            self.ignore_key = ignore.to_vec();
        }
        box_blur(gray, w, h, &mut self.blur);

        let mut sum = 0f64;
        let mut sq = 0f64;
        let mut bg_sum = 0f64;
        for i in 0..n {
            let f = self.blur[i] as f64;
            sum += f;
            sq += f * f;
            bg_sum += self.bg[i] as f64;
        }
        let mean = sum / n as f64;
        let bg_mean = bg_sum / n as f64;
        let std = ((sq / n as f64) - mean * mean).max(0.0).sqrt();
        // Compensate for gradual global brightness shifts (webcam auto-exposure,
        // monitor glow, clouds passing a window) so lighting swings don't look like people.
        let raw_delta = (mean - bg_mean) as f32;
        let illum_shift = if raw_delta.abs() < 35.0 { raw_delta * 0.85 } else { 0.0 };

        let mut ns = 0f32;
        let mut nc = 0u32;
        let mut i = 0;
        while i < n {
            let d = ((self.blur[i] as f32 - self.bg[i]) - illum_shift).abs();
            if d < 20.0 {
                ns += d;
                nc += 1;
            }
            i += 7;
        }
        if nc > 0 {
            self.noise = self.noise * 0.95 + (ns / nc as f32) * 0.05;
        }
        let base = [36.0f32, 28.0, 21.0, 15.0, 11.0][(sensitivity.clamp(1, 5) - 1) as usize];
        let thr = base + 2.0 * self.noise;

        let mut count = 0usize;
        for i in 0..n {
            let d = ((self.blur[i] as f32 - self.bg[i]) - illum_shift).abs();
            let pthr = thr + 2.5 * self.bg_std.get(i).copied().unwrap_or(0.0);
            let on = d > pthr && self.ignore[i] == 0;
            self.raw[i] = on as u8;
            if on {
                count += 1;
            }
        }
        let total = count as f32 / n as f32;
        if total > 0.42 || raw_delta.abs() >= 35.0 || self.settle > 0 {
            let light = (total > 0.42 || raw_delta.abs() >= 35.0) && self.settle == 0;
            if self.settle > 0 {
                self.settle -= 1;
            }
            for i in 0..n {
                self.bg[i] = self.blur[i] as f32;
            }
            for m in self.mask.iter_mut() {
                *m = 0;
            }
            return MotionOut {
                light_change: light,
                mean: mean as f32,
                std: std as f32,
                valid: false,
                ..Default::default()
            };
        }
        clean(&self.raw, w, h, &mut self.mask);

        let (mut door_px, mut door_on, mut room_px, mut room_on) = (0usize, 0usize, 0usize, 0usize);
        for y in 0..h {
            let fy = (y as f32 + 0.5) / h as f32;
            for x in 0..w {
                let i = y * w + x;
                if self.ignore[i] != 0 {
                    continue;
                }
                let fx = (x as f32 + 0.5) / w as f32;
                let on = self.mask[i] != 0;
                let in_door = match door {
                    Some(d) => d.contains(fx, fy),
                    None => false,
                };
                if in_door {
                    door_px += 1;
                    door_on += on as usize;
                } else {
                    room_px += 1;
                    room_on += on as usize;
                }
            }
        }
        for i in 0..n {
            let a = if self.mask[i] != 0 { 0.003 } else { 0.045 };
            self.bg[i] += a * (self.blur[i] as f32 - self.bg[i]);
        }
        MotionOut {
            door_frac: door_on as f32 / door_px.max(1) as f32,
            room_frac: room_on as f32 / room_px.max(1) as f32,
            total_frac: (door_on + room_on) as f32 / (door_px + room_px).max(1) as f32,
            light_change: false,
            mean: mean as f32,
            std: std as f32,
            valid: true,
        }
    }

    /// Share of moving pixels in each cell of a `gw` by `gh` grid.
    pub fn grid(&self, gw: usize, gh: usize) -> Vec<f32> {
        let mut g = vec![0f32; gw * gh];
        if !self.ready || self.mask.len() != self.w * self.h || gw == 0 || gh == 0 {
            return g;
        }
        let mut cnt = vec![0u32; gw * gh];
        for y in 0..self.h {
            let gy = (y * gh / self.h).min(gh - 1);
            for x in 0..self.w {
                let gx = (x * gw / self.w).min(gw - 1);
                let k = gy * gw + gx;
                cnt[k] += 1;
                if self.mask[y * self.w + x] != 0 {
                    g[k] += 1.0;
                }
            }
        }
        for k in 0..g.len() {
            g[k] /= cnt[k].max(1) as f32;
        }
        g
    }
}

fn box_blur(src: &[u8], w: usize, h: usize, out: &mut [u8]) {
    if w < 3 || h < 3 {
        out[..w * h].copy_from_slice(&src[..w * h]);
        return;
    }
    for y in 0..h {
        for x in 0..w {
            if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                out[y * w + x] = src[y * w + x];
                continue;
            }
            let mut s = 0u32;
            for dy in 0..3 {
                let row = (y + dy - 1) * w;
                for dx in 0..3 {
                    s += src[row + x + dx - 1] as u32;
                }
            }
            out[y * w + x] = (s / 9) as u8;
        }
    }
}

/// Keeps a moving pixel only when at least three of its neighbours moved
/// too. That removes sensor noise speckle without eating real shapes.
fn clean(raw: &[u8], w: usize, h: usize, out: &mut [u8]) {
    for v in out.iter_mut() {
        *v = 0;
    }
    if w < 3 || h < 3 {
        return;
    }
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            if raw[i] == 0 {
                continue;
            }
            let mut n = 0;
            for dy in 0..3 {
                let row = (y + dy - 1) * w;
                for dx in 0..3 {
                    n += raw[row + x + dx - 1] as u32;
                }
            }
            if n >= 5 {
                out[i] = 1;
            }
        }
    }
}
