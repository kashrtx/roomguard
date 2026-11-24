//! Follows people from frame to frame.
//!
//! Motion pixels are grouped into blobs, and each person gets a track with a
//! small Kalman filter that estimates where they are and how fast they move.
//! When someone disappears, the filter's velocity says where they were going,
//! and the prediction fades the speed out over about a second, because people
//! slow down and turn rather than flying off in a straight line.

use std::collections::VecDeque;
use std::time::Instant;

/// Process noise: how hard people can speed up or turn, in views per second squared.
const Q: f32 = 2.5;
/// Measurement noise of a blob centre.
const R: f32 = 0.0004;

#[derive(Clone, Copy, Debug)]
pub struct Blob {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub px: u32,
}

#[derive(Clone, Copy, Debug)]
struct Kf {
    x: f32,
    v: f32,
    p00: f32,
    p01: f32,
    p11: f32,
}

impl Kf {
    fn new(x: f32) -> Self {
        Kf { x, v: 0.0, p00: 0.002, p01: 0.0, p11: 0.5 }
    }

    fn predict(&mut self, dt: f32) {
        self.x += self.v * dt;
        let dt2 = dt * dt;
        let p00 = self.p00 + dt * (2.0 * self.p01 + dt * self.p11) + Q * dt2 * dt2 * 0.25;
        let p01 = self.p01 + dt * self.p11 + Q * dt2 * dt * 0.5;
        let p11 = self.p11 + Q * dt2;
        self.p00 = p00;
        self.p01 = p01;
        self.p11 = p11;
    }

    fn update(&mut self, z: f32) {
        let s = self.p00 + R;
        let k0 = self.p00 / s;
        let k1 = self.p01 / s;
        let y = z - self.x;
        self.x += k0 * y;
        self.v += k1 * y;
        let p00 = (1.0 - k0) * self.p00;
        let p01 = (1.0 - k0) * self.p01;
        let p11 = self.p11 - k1 * self.p01;
        self.p00 = p00;
        self.p01 = p01;
        self.p11 = p11.max(1e-6);
        self.v = self.v.clamp(-2.5, 2.5);
    }
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: u32,
    kx: Kf,
    ky: Kf,
    pub w: f32,
    pub h: f32,
    pub hits: u32,
    pub misses: u32,
    pub born: Instant,
    pub last_seen: Instant,
    pub confirmed: bool,
    pub path: VecDeque<(f32, f32)>,
    pub first_foot: (f32, f32),
    pub label: Option<String>,
    pub known: bool,
    pub travelled: f32,
}

impl Track {
    fn new(id: u32, b: &Blob, now: Instant) -> Self {
        let foot = (b.cx, b.cy + b.h * 0.5);
        let mut path = VecDeque::new();
        path.push_back(foot);
        Track {
            id,
            kx: Kf::new(b.cx),
            ky: Kf::new(b.cy),
            w: b.w,
            h: b.h,
            hits: 1,
            misses: 0,
            born: now,
            last_seen: now,
            confirmed: false,
            path,
            first_foot: foot,
            label: None,
            known: false,
            travelled: 0.0,
        }
    }

    pub fn pos(&self) -> (f32, f32) {
        (self.kx.x, self.ky.x)
    }

    pub fn vel(&self) -> (f32, f32) {
        (self.kx.v, self.ky.v)
    }

    pub fn speed(&self) -> f32 {
        (self.kx.v * self.kx.v + self.ky.v * self.ky.v).sqrt()
    }

    pub fn foot(&self) -> (f32, f32) {
        (self.kx.x, (self.ky.x + self.h * 0.5).min(1.0))
    }

    pub fn unseen(&self, now: Instant) -> f32 {
        now.saturating_duration_since(self.last_seen).as_secs_f32()
    }

    /// Where this person probably is `t` seconds after the last sighting.
    pub fn predict(&self, t: f32) -> (f32, f32) {
        let tau = 0.8;
        let k = tau * (1.0 - (-t.max(0.0) / tau).exp());
        (self.kx.x + self.kx.v * k, self.ky.x + self.ky.v * k)
    }

    /// A few points along the predicted path, for drawing.
    pub fn predicted_path(&self, steps: usize, horizon: f32) -> Vec<(f32, f32)> {
        (1..=steps)
            .map(|i| self.predict(horizon * i as f32 / steps as f32))
            .collect()
    }

    fn finish(&self) -> Finished {
        Finished {
            id: self.id,
            first: self.first_foot,
            last: self.foot(),
            last_vel: self.vel(),
            path: self.path.iter().copied().collect(),
            secs: self.last_seen.saturating_duration_since(self.born).as_secs_f32(),
            known: self.known,
        }
    }
}

/// A track that ended, handed to the room map so it can learn from it.
#[derive(Clone, Debug)]
pub struct Finished {
    pub id: u32,
    pub first: (f32, f32),
    pub last: (f32, f32),
    pub last_vel: (f32, f32),
    pub path: Vec<(f32, f32)>,
    pub secs: f32,
    pub known: bool,
}

pub struct Tracker {
    pub tracks: Vec<Track>,
    next_id: u32,
    pub finished: Vec<Finished>,
    last: Option<Instant>,
}

impl Tracker {
    pub fn new() -> Self {
        Tracker { tracks: Vec::new(), next_id: 1, finished: Vec::new(), last: None }
    }

    pub fn clear(&mut self) {
        self.tracks.clear();
        self.last = None;
    }

    pub fn update(&mut self, blobs: &[Blob], now: Instant) {
        let dt = self
            .last
            .map(|t| now.saturating_duration_since(t).as_secs_f32())
            .unwrap_or(0.05)
            .clamp(0.001, 0.5);
        self.last = Some(now);
        for t in self.tracks.iter_mut() {
            t.kx.predict(dt);
            t.ky.predict(dt);
        }

        // Greedy nearest matching inside a gate that grows with size and speed.
        let mut pairs: Vec<(f32, usize, usize)> = Vec::new();
        for (ti, t) in self.tracks.iter().enumerate() {
            let (px, py) = t.pos();
            for (bi, b) in blobs.iter().enumerate() {
                let dx = b.cx - px;
                let dy = b.cy - py;
                let d = (dx * dx + dy * dy).sqrt();
                let gate = 0.06 + 0.6 * t.w.max(t.h) + t.speed() * dt * 2.0;
                if d < gate {
                    pairs.push((d / gate, ti, bi));
                }
            }
        }
        pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut t_used = vec![false; self.tracks.len()];
        let mut b_used = vec![false; blobs.len()];
        for (_, ti, bi) in pairs {
            if t_used[ti] || b_used[bi] {
                continue;
            }
            t_used[ti] = true;
            b_used[bi] = true;
            let b = blobs[bi];
            let t = &mut self.tracks[ti];
            let before = t.foot();
            t.kx.update(b.cx);
            t.ky.update(b.cy);
            t.w = t.w * 0.6 + b.w * 0.4;
            t.h = t.h * 0.6 + b.h * 0.4;
            t.hits += 1;
            t.misses = 0;
            t.last_seen = now;
            if t.hits >= 3 {
                t.confirmed = true;
            }
            let foot = t.foot();
            let dx = foot.0 - before.0;
            let dy = foot.1 - before.1;
            t.travelled += (dx * dx + dy * dy).sqrt();
            t.path.push_back(foot);
            if t.path.len() > 160 {
                t.path.pop_front();
            }
        }
        for (ti, t) in self.tracks.iter_mut().enumerate() {
            if !t_used[ti] {
                t.misses += 1;
            }
        }
        for (bi, b) in blobs.iter().enumerate() {
            if !b_used[bi] {
                self.tracks.push(Track::new(self.next_id, b, now));
                self.next_id = self.next_id.wrapping_add(1).max(1);
            }
        }

        let mut keep = Vec::with_capacity(self.tracks.len());
        for t in self.tracks.drain(..) {
            let limit = if t.confirmed { 1.5 } else { 0.35 };
            if t.unseen(now) > limit {
                if t.confirmed {
                    self.finished.push(t.finish());
                }
            } else {
                keep.push(t);
            }
        }
        self.tracks = keep;
    }

    /// The person worth paying attention to: unknown people first, then the
    /// biggest and most recently seen.
    pub fn primary(&self, now: Instant) -> Option<&Track> {
        self.tracks
            .iter()
            .filter(|t| t.confirmed)
            .max_by(|a, b| {
                let sa = score(a, now);
                let sb = score(b, now);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

fn score(t: &Track, now: Instant) -> f32 {
    let fresh = 1.0 / (1.0 + t.unseen(now) * 4.0);
    let unknown = if t.known { 0.6 } else { 1.0 };
    (t.w * t.h).sqrt() * fresh * unknown
}

/// Groups moving pixels into blobs, then merges pieces of the same person.
pub fn find_blobs(mask: &[u8], w: usize, h: usize, min_px: u32) -> Vec<Blob> {
    if w == 0 || h == 0 || mask.len() < w * h {
        return Vec::new();
    }
    let mut seen = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<Blob> = Vec::new();
    for start in 0..w * h {
        if mask[start] == 0 || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (w, h, 0usize, 0usize, 0u32);
        while let Some(j) = stack.pop() {
            let x = j % w;
            let y = j / w;
            n += 1;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            let xs = x.saturating_sub(1);
            let ys = y.saturating_sub(1);
            for ny in ys..=(y + 1).min(h - 1) {
                for nx in xs..=(x + 1).min(w - 1) {
                    let k = ny * w + nx;
                    if mask[k] != 0 && !seen[k] {
                        seen[k] = true;
                        stack.push(k);
                    }
                }
            }
        }
        if n >= min_px {
            out.push(Blob {
                cx: (x0 + x1 + 1) as f32 * 0.5 / w as f32,
                cy: (y0 + y1 + 1) as f32 * 0.5 / h as f32,
                w: (x1 - x0 + 1) as f32 / w as f32,
                h: (y1 - y0 + 1) as f32 / h as f32,
                px: n,
            });
        }
    }
    merge_close(out)
}

fn merge_close(mut v: Vec<Blob>) -> Vec<Blob> {
    loop {
        let mut merged = false;
        'outer: for i in 0..v.len() {
            for j in (i + 1)..v.len() {
                if near(&v[i], &v[j], 0.035) {
                    let m = union(&v[i], &v[j]);
                    v[i] = m;
                    v.remove(j);
                    merged = true;
                    break 'outer;
                }
            }
        }
        if !merged {
            break;
        }
    }
    v
}

fn near(a: &Blob, b: &Blob, pad: f32) -> bool {
    let ax0 = a.cx - a.w * 0.5 - pad;
    let ax1 = a.cx + a.w * 0.5 + pad;
    let ay0 = a.cy - a.h * 0.5 - pad;
    let ay1 = a.cy + a.h * 0.5 + pad;
    let bx0 = b.cx - b.w * 0.5;
    let bx1 = b.cx + b.w * 0.5;
    let by0 = b.cy - b.h * 0.5;
    let by1 = b.cy + b.h * 0.5;
    ax0 < bx1 && bx0 < ax1 && ay0 < by1 && by0 < ay1
}

fn union(a: &Blob, b: &Blob) -> Blob {
    let x0 = (a.cx - a.w * 0.5).min(b.cx - b.w * 0.5);
    let x1 = (a.cx + a.w * 0.5).max(b.cx + b.w * 0.5);
    let y0 = (a.cy - a.h * 0.5).min(b.cy - b.h * 0.5);
    let y1 = (a.cy + a.h * 0.5).max(b.cy + b.h * 0.5);
    Blob { cx: (x0 + x1) * 0.5, cy: (y0 + y1) * 0.5, w: x1 - x0, h: y1 - y0, px: a.px + b.px }
}
