//! The room map. RoomGuard can't measure depth with one camera, so instead
//! of a 3D floor plan it learns the room the way a security operator does by
//! watching it: where people walk, where they come in and leave, which spots
//! move on their own (a TV, a fan, curtains), and what the door looks like
//! when it's closed.
//!
//! Everything is stored on a 32 by 24 grid in map space. For a fixed camera
//! map space is simply the view. For a PTZ camera it's the whole range the
//! camera can turn through, so the map stays put while the camera moves.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::frame;
use super::track::Finished;
use crate::config::NRect;

pub const GW: usize = 32;
pub const GH: usize = 24;
const MAX_POINTS: usize = 400;

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Portal {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub uses: u32,
    /// 0 inside the view, 1 left edge, 2 right edge, 3 top, 4 bottom.
    pub edge: u8,
    pub dismissed: bool,
}

impl Portal {
    pub fn rect(&self) -> NRect {
        NRect::new(self.x, self.y, self.w, self.h)
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
    pub fn describe(&self) -> &'static str {
        match self.edge {
            1 => "People leave the view on the left here",
            2 => "People leave the view on the right here",
            3 => "People leave the view at the top here",
            4 => "People come and go at the bottom of the view here",
            _ => "Looks like a doorway",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RoomMap {
    pub heat: Vec<f32>,
    pub floor: Vec<f32>,
    pub nuisance: Vec<f32>,
    pub near_tracks: Vec<f32>,
    pub enters: Vec<[f32; 2]>,
    pub exits: Vec<[f32; 2]>,
    /// Per cell: how often people moved to each neighbour (index 4 = stayed).
    pub moves: Vec<[f32; 9]>,
    pub portals: Vec<Portal>,
    pub tracks: u32,
    pub frames: u64,
}

impl RoomMap {
    pub fn ensure(&mut self) {
        let n = GW * GH;
        for v in [&mut self.heat, &mut self.floor, &mut self.nuisance, &mut self.near_tracks] {
            if v.len() != n {
                *v = vec![0.0; n];
            }
        }
        if self.moves.len() != n {
            self.moves = vec![[0.0; 9]; n];
        }
    }

    pub fn clear(&mut self) {
        *self = RoomMap::default();
        self.ensure();
    }

    pub fn cell(x: f32, y: f32) -> usize {
        let cx = (x.clamp(0.0, 0.9999) * GW as f32) as usize;
        let cy = (y.clamp(0.0, 0.9999) * GH as f32) as usize;
        cy * GW + cx
    }

    pub fn cell_center(c: usize) -> (f32, f32) {
        (((c % GW) as f32 + 0.5) / GW as f32, ((c / GW) as f32 + 0.5) / GH as f32)
    }

    /// Called every frame with the motion share per cell (already in map
    /// space) and where the tracked people are.
    pub fn observe(&mut self, motion: &[f32], people: &[(f32, f32)]) {
        self.ensure();
        if motion.len() != GW * GH {
            return;
        }
        self.frames += 1;
        let decay = 0.99985;
        for k in 0..GW * GH {
            self.heat[k] = self.heat[k] * decay + motion[k];
            self.near_tracks[k] *= decay;
            self.nuisance[k] *= decay;
        }
        for &(x, y) in people {
            let c = Self::cell(x, y) as i32;
            let (cx, cy) = (c % GW as i32, c / GW as i32);
            for dy in -2..=2 {
                for dx in -2..=2 {
                    let nx = cx + dx;
                    let ny = cy + dy;
                    if nx >= 0 && ny >= 0 && nx < GW as i32 && ny < GH as i32 {
                        self.near_tracks[(ny * GW as i32 + nx) as usize] += 1.0;
                    }
                }
            }
        }
        for k in 0..GW * GH {
            if motion[k] > 0.15 && self.near_tracks[k] < 0.5 {
                self.nuisance[k] += motion[k];
            }
        }
    }

    /// Learns from a finished walk through the room.
    pub fn learn(&mut self, f: &Finished) {
        if f.secs < 0.6 || f.path.len() < 5 {
            return;
        }
        self.ensure();
        push_cap(&mut self.enters, [f.first.0, f.first.1]);
        push_cap(&mut self.exits, [f.last.0, f.last.1]);
        for &(x, y) in &f.path {
            self.floor[Self::cell(x, y)] += 1.0;
        }
        for pair in f.path.windows(2) {
            let a = Self::cell(pair[0].0, pair[0].1);
            let b = Self::cell(pair[1].0, pair[1].1);
            let d = direction(a, b);
            self.moves[a][d] += 1.0;
        }
        self.tracks += 1;
        self.rebuild_portals();
    }

    /// Finds the places people keep appearing and disappearing. A cluster in
    /// the middle of the view is almost always a door. A cluster at the edge
    /// is where people walk out of sight.
    pub fn rebuild_portals(&mut self) {
        const CW: usize = 12;
        const CH: usize = 9;
        let pts: Vec<[f32; 2]> = self.enters.iter().chain(self.exits.iter()).copied().collect();
        if pts.len() < 4 {
            return;
        }
        let cell_of = |p: &[f32; 2]| -> usize {
            let cx = (p[0].clamp(0.0, 0.9999) * CW as f32) as usize;
            let cy = (p[1].clamp(0.0, 0.9999) * CH as f32) as usize;
            cy * CW + cx
        };
        let mut votes = vec![0u32; CW * CH];
        for p in &pts {
            votes[cell_of(p)] += 1;
        }
        let mut used = vec![false; pts.len()];
        let mut found: Vec<Portal> = Vec::new();
        for _ in 0..12 {
            let mut best = 0usize;
            let mut count = 0u32;
            for (i, &v) in votes.iter().enumerate() {
                if v > count {
                    count = v;
                    best = i;
                }
            }
            if count < 3 || found.len() >= 6 {
                break;
            }
            let bx = ((best % CW) as f32 + 0.5) / CW as f32;
            let by = ((best / CW) as f32 + 0.5) / CH as f32;
            let mut members = Vec::new();
            for (i, p) in pts.iter().enumerate() {
                if used[i] {
                    continue;
                }
                let d = ((p[0] - bx).powi(2) + (p[1] - by).powi(2)).sqrt();
                if d < 0.14 {
                    members.push(i);
                }
            }
            votes[best] = 0;
            if members.len() < 3 {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1) = (1.0f32, 1.0f32, 0.0f32, 0.0f32);
            for &i in &members {
                used[i] = true;
                let c = cell_of(&pts[i]);
                votes[c] = votes[c].saturating_sub(1);
                x0 = x0.min(pts[i][0]);
                y0 = y0.min(pts[i][1]);
                x1 = x1.max(pts[i][0]);
                y1 = y1.max(pts[i][1]);
            }
            let (mut x0, mut y0, mut x1, mut y1) = (x0 - 0.03, y0 - 0.03, x1 + 0.03, y1 + 0.03);
            if x1 - x0 < 0.10 {
                let c = (x0 + x1) * 0.5;
                x0 = c - 0.05;
                x1 = c + 0.05;
            }
            if y1 - y0 < 0.16 {
                let c = (y0 + y1) * 0.5;
                y0 = c - 0.08;
                y1 = c + 0.08;
            }
            let r = NRect::from_points(x0, y0, x1, y1);
            let edge = if r.x < 0.05 {
                1
            } else if r.x + r.w > 0.95 {
                2
            } else if r.y < 0.05 {
                3
            } else if r.y + r.h > 0.95 {
                4
            } else {
                0
            };
            found.push(Portal { x: r.x, y: r.y, w: r.w, h: r.h, uses: members.len() as u32, edge, dismissed: false });
        }
        for p in found.iter_mut() {
            if self.portals.iter().any(|o| o.dismissed && o.rect().overlap(&p.rect()) > 0.3) {
                p.dismissed = true;
            }
        }
        self.portals = found;
    }

    /// Places a person heading this way is most likely to go next, best first.
    pub fn likely_exits(&self, x: f32, y: f32, vx: f32, vy: f32) -> Vec<(f32, f32, f32)> {
        let speed = (vx * vx + vy * vy).sqrt();
        let mut out = Vec::new();
        for p in &self.portals {
            if p.dismissed {
                continue;
            }
            let (cx, cy) = p.center();
            let dx = cx - x;
            let dy = cy - y;
            let dist = (dx * dx + dy * dy).sqrt().max(0.02);
            let align = if speed > 0.05 { (dx * vx + dy * vy) / (dist * speed) } else { 0.0 };
            let prior = (p.uses as f32).ln_1p();
            let score = prior * (1.0 + 1.5 * align.max(-0.5)) / (0.3 + dist);
            out.push((cx, cy, score));
        }
        out.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    /// Follows the room's learned walking habits from a point, blended with
    /// the current heading. Returns up to `n` likely next positions.
    pub fn next_steps(&self, x: f32, y: f32, vx: f32, vy: f32, n: usize) -> Vec<(f32, f32)> {
        if self.moves.len() != GW * GH {
            return Vec::new();
        }
        let mut c = Self::cell(x, y);
        let mut hx = vx;
        let mut hy = vy;
        let mut out = Vec::new();
        for _ in 0..n {
            let m = self.moves[c];
            let total: f32 = m.iter().sum();
            let hl = (hx * hx + hy * hy).sqrt();
            let mut best = 4usize;
            let mut best_s = 0.05f32;
            for d in 0..9 {
                if d == 4 {
                    continue;
                }
                let dx = (d % 3) as f32 - 1.0;
                let dy = (d / 3) as f32 - 1.0;
                let learned = if total > 0.0 { m[d] / total } else { 0.0 };
                let heading = if hl > 0.01 {
                    ((dx * hx + dy * hy) / ((dx * dx + dy * dy).sqrt() * hl)).max(0.0)
                } else {
                    0.0
                };
                let s = learned * 1.2 + heading * 0.8;
                if s > best_s {
                    best_s = s;
                    best = d;
                }
            }
            if best == 4 {
                break;
            }
            let nx = (c % GW) as i32 + (best % 3) as i32 - 1;
            let ny = (c / GW) as i32 + (best / 3) as i32 - 1;
            if nx < 0 || ny < 0 || nx >= GW as i32 || ny >= GH as i32 {
                break;
            }
            hx = (best % 3) as f32 - 1.0;
            hy = (best / 3) as f32 - 1.0;
            c = ny as usize * GW + nx as usize;
            out.push(Self::cell_center(c));
        }
        out
    }

    /// The best doorway candidate, if the room has seen enough foot traffic.
    pub fn suggest_doorway(&self) -> Option<NRect> {
        let mut best: Option<&Portal> = None;
        for p in &self.portals {
            if p.dismissed || p.uses < 3 {
                continue;
            }
            let rank = |q: &Portal| q.uses as f32 * if q.edge == 0 { 1.6 } else { 1.0 };
            if best.map_or(true, |b| rank(p) > rank(b)) {
                best = Some(p);
            }
        }
        best.map(|p| p.rect())
    }

    /// Spots that keep moving without anyone near them.
    pub fn suggest_ignore(&self) -> Vec<NRect> {
        let n = GW * GH;
        if self.nuisance.len() != n || self.frames < 3000 {
            return Vec::new();
        }
        let hot: Vec<bool> = (0..n).map(|k| self.nuisance[k] > 40.0 && self.floor[k] < 2.0).collect();
        let mut seen = vec![false; n];
        let mut out = Vec::new();
        for start in 0..n {
            if !hot[start] || seen[start] {
                continue;
            }
            let mut stack = vec![start];
            seen[start] = true;
            let (mut x0, mut y0, mut x1, mut y1, mut count) = (GW, GH, 0usize, 0usize, 0usize);
            while let Some(c) = stack.pop() {
                let (cx, cy) = (c % GW, c / GW);
                count += 1;
                x0 = x0.min(cx);
                y0 = y0.min(cy);
                x1 = x1.max(cx);
                y1 = y1.max(cy);
                let nbrs = [
                    (cx as i32 - 1, cy as i32),
                    (cx as i32 + 1, cy as i32),
                    (cx as i32, cy as i32 - 1),
                    (cx as i32, cy as i32 + 1),
                ];
                for (nx, ny) in nbrs {
                    if nx >= 0 && ny >= 0 && nx < GW as i32 && ny < GH as i32 {
                        let k = ny as usize * GW + nx as usize;
                        if hot[k] && !seen[k] {
                            seen[k] = true;
                            stack.push(k);
                        }
                    }
                }
            }
            if count >= 2 {
                out.push(NRect::new(
                    x0 as f32 / GW as f32,
                    y0 as f32 / GH as f32,
                    (x1 - x0 + 1) as f32 / GW as f32,
                    (y1 - y0 + 1) as f32 / GH as f32,
                ));
            }
            if out.len() >= 3 {
                break;
            }
        }
        out
    }

    pub fn dismiss_portal(&mut self, index: usize) {
        if let Some(p) = self.portals.get_mut(index) {
            p.dismissed = true;
        }
    }
}

fn push_cap(v: &mut Vec<[f32; 2]>, p: [f32; 2]) {
    v.push(p);
    if v.len() > MAX_POINTS {
        let extra = v.len() - MAX_POINTS;
        v.drain(0..extra);
    }
}

fn direction(a: usize, b: usize) -> usize {
    let (ax, ay) = ((a % GW) as i32, (a / GW) as i32);
    let (bx, by) = ((b % GW) as i32, (b / GW) as i32);
    let dx = (bx - ax).signum();
    let dy = (by - ay).signum();
    ((dy + 1) * 3 + (dx + 1)) as usize
}

/// Watches the doorway like a door contact. It remembers what the doorway
/// looks like when the door is shut (as an edge pattern, so lighting changes
/// don't fool it), and reports the door open when that pattern changes and
/// stays changed after the movement stops.
pub struct DoorWatch {
    reference: Option<Vec<f32>>,
    ref_rect: Option<NRect>,
    still_since: Option<Instant>,
    flip_since: Option<Instant>,
    pub open: bool,
    pub score: f32,
    pub learned: bool,
}

impl DoorWatch {
    pub fn new() -> Self {
        DoorWatch { reference: None, ref_rect: None, still_since: None, flip_since: None, open: false, score: 1.0, learned: false }
    }

    pub fn relearn(&mut self) {
        self.reference = None;
        self.open = false;
        self.learned = false;
        self.flip_since = None;
    }

    /// Returns `Some(open)` when the door state changes.
    pub fn update(&mut self, gray: &[u8], w: usize, h: usize, rect: NRect, moving: bool, now: Instant) -> Option<bool> {
        if self.ref_rect != Some(rect) {
            self.relearn();
            self.ref_rect = Some(rect);
        }
        if moving {
            self.still_since = None;
            self.flip_since = None;
            return None;
        }
        let still = *self.still_since.get_or_insert(now);
        let patch = signature(gray, w, h, rect)?;
        if self.reference.is_none() {
            if now.saturating_duration_since(still).as_secs_f32() > 4.0 {
                self.reference = Some(patch);
                self.learned = true;
                self.open = false;
            }
            return None;
        }
        let score = self.reference.as_ref().map(|r| ncc(r, &patch)).unwrap_or(1.0);
        self.score = score;
        let flip = if self.open { score > 0.75 } else { score < 0.55 };
        let mut changed = None;
        if flip {
            let since = *self.flip_since.get_or_insert(now);
            if now.saturating_duration_since(since).as_secs_f32() > 1.2 {
                self.open = !self.open;
                self.flip_since = None;
                changed = Some(self.open);
            }
        } else {
            self.flip_since = None;
        }
        if !self.open && score > 0.85 {
            if let Some(r) = self.reference.as_mut() {
                for (a, b) in r.iter_mut().zip(patch.iter()) {
                    *a = *a * 0.98 + *b * 0.02;
                }
            }
        }
        changed
    }
}

fn signature(gray: &[u8], w: usize, h: usize, rect: NRect) -> Option<Vec<f32>> {
    const SW: usize = 24;
    const SH: usize = 32;
    let px = rect.x * w as f32;
    let py = rect.y * h as f32;
    let pw = (rect.w * w as f32).max(4.0);
    let ph = (rect.h * h as f32).max(4.0);
    let patch = frame::crop_resize(gray, w, h, px, py, pw, ph, SW, SH);
    let mut g = Vec::with_capacity((SW - 2) * (SH - 2));
    for y in 1..SH - 1 {
        for x in 1..SW - 1 {
            let gx = patch[y * SW + x + 1] as f32 - patch[y * SW + x - 1] as f32;
            let gy = patch[(y + 1) * SW + x] as f32 - patch[(y - 1) * SW + x] as f32;
            g.push(gx.abs() + gy.abs());
        }
    }
    let n = g.len() as f32;
    let mean = g.iter().sum::<f32>() / n;
    let var = g.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
    let std = var.sqrt();
    if std < 1.0 {
        return None;
    }
    for v in g.iter_mut() {
        *v = (*v - mean) / std;
    }
    Some(g)
}

fn ncc(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 1.0;
    }
    let (ma, mb) = (a[..n].iter().sum::<f32>() / n as f32, b[..n].iter().sum::<f32>() / n as f32);
    let mut num = 0.0;
    let mut da = 0.0;
    let mut db = 0.0;
    for i in 0..n {
        let x = a[i] - ma;
        let y = b[i] - mb;
        num += x * y;
        da += x * x;
        db += y * y;
    }
    num / (da * db).sqrt().max(1e-6)
}
