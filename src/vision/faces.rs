//! Face detection and recognition, all local, nothing downloaded.
//!
//! Detection uses OpenCV's LBP frontal face cascade (converted to a compact
//! binary by tools/convert_cascade.py). The evaluation below matches OpenCV's
//! own algorithm step for step, and was checked against known face positions
//! before it was written in Rust.
//!
//! Recognition uses LBP histograms (LBPH), a classic method that works well
//! when the same camera sees the same person in the same room. Each person's
//! match threshold is worked out from their own enrollment photos.

use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use super::frame;
use crate::util;

pub const FACE: usize = 64;
const GRID: usize = 7;
const BINS: usize = 59;
pub const DIM: usize = GRID * GRID * BINS;

#[derive(Clone, Copy, Debug)]
pub struct FaceRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub votes: u32,
}

struct Weak {
    feat: usize,
    subset: [i32; 8],
    hit: f32,
    miss: f32,
}

struct Stage {
    thr: f32,
    weaks: Vec<Weak>,
}

pub struct Cascade {
    win_w: usize,
    win_h: usize,
    feats: Vec<[i32; 4]>,
    stages: Vec<Stage>,
}

struct Reader<'a> {
    b: &'a [u8],
    o: usize,
}

impl<'a> Reader<'a> {
    fn take4(&mut self) -> Option<[u8; 4]> {
        let s = self.b.get(self.o..self.o + 4)?;
        self.o += 4;
        Some([s[0], s[1], s[2], s[3]])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take4().map(u32::from_le_bytes)
    }
    fn i32(&mut self) -> Option<i32> {
        self.take4().map(i32::from_le_bytes)
    }
    fn f32(&mut self) -> Option<f32> {
        self.take4().map(f32::from_le_bytes)
    }
}

fn bit(on: bool, shift: u32) -> u32 {
    (on as u32) << shift
}

impl Cascade {
    pub fn load(b: &[u8]) -> Option<Cascade> {
        if !b.starts_with(b"RGLBP1") {
            return None;
        }
        let mut r = Reader { b, o: 8 };
        let win_w = r.u32()? as usize;
        let win_h = r.u32()? as usize;
        let nf = r.u32()? as usize;
        let mut feats = Vec::with_capacity(nf);
        for _ in 0..nf {
            feats.push([r.i32()?, r.i32()?, r.i32()?, r.i32()?]);
        }
        let ns = r.u32()? as usize;
        let mut stages = Vec::with_capacity(ns);
        for _ in 0..ns {
            let thr = r.f32()? - 1e-5;
            let nw = r.u32()? as usize;
            let mut weaks = Vec::with_capacity(nw);
            for _ in 0..nw {
                let feat = r.u32()? as usize;
                let mut subset = [0i32; 8];
                for s in subset.iter_mut() {
                    *s = r.i32()?;
                }
                let hit = r.f32()?;
                let miss = r.f32()?;
                if feat >= feats.len() {
                    return None;
                }
                weaks.push(Weak { feat, subset, hit, miss });
            }
            stages.push(Stage { thr, weaks });
        }
        if win_w == 0 || win_h == 0 || stages.is_empty() {
            return None;
        }
        Some(Cascade { win_w, win_h, feats, stages })
    }

    fn eval(&self, ii: &[i32], stride: usize, x: usize, y: usize) -> bool {
        for st in &self.stages {
            let mut sum = 0.0f32;
            for wk in &st.weaks {
                let f = self.feats[wk.feat];
                let fx = x as i32 + f[0];
                let fy = y as i32 + f[1];
                let mut p = [0i32; 16];
                for j in 0..4 {
                    let row = (fy + j as i32 * f[3]) as usize * stride;
                    for i in 0..4 {
                        p[j * 4 + i] = ii[row + (fx + i as i32 * f[2]) as usize];
                    }
                }
                let cell = |cx: usize, cy: usize| -> i32 {
                    p[cy * 4 + cx] - p[cy * 4 + cx + 1] - p[(cy + 1) * 4 + cx] + p[(cy + 1) * 4 + cx + 1]
                };
                let c = cell(1, 1);
                let code = bit(cell(0, 0) >= c, 7)
                    | bit(cell(1, 0) >= c, 6)
                    | bit(cell(2, 0) >= c, 5)
                    | bit(cell(2, 1) >= c, 4)
                    | bit(cell(2, 2) >= c, 3)
                    | bit(cell(1, 2) >= c, 2)
                    | bit(cell(0, 2) >= c, 1)
                    | bit(cell(0, 1) >= c, 0);
                let word = wk.subset[(code >> 5) as usize] as u32;
                sum += if (word >> (code & 31)) & 1 == 1 { wk.hit } else { wk.miss };
            }
            if sum < st.thr {
                return false;
            }
        }
        true
    }

    /// Finds faces at least `min_size` pixels wide.
    pub fn detect(&self, gray: &[u8], w: usize, h: usize, min_size: f32, scale_factor: f32, min_neighbors: u32) -> Vec<FaceRect> {
        let mut raw: Vec<[f32; 4]> = Vec::new();
        if gray.len() < w * h {
            return Vec::new();
        }
        let mut factor = 1.0f32;
        loop {
            let ww = (self.win_w as f32 * factor).round();
            let wh = (self.win_h as f32 * factor).round();
            if ww > w as f32 || wh > h as f32 {
                break;
            }
            let sw = (w as f32 / factor).round() as usize;
            let sh = (h as f32 / factor).round() as usize;
            if sw < self.win_w || sh < self.win_h {
                break;
            }
            if ww >= min_size {
                let small = if factor == 1.0 { gray[..w * h].to_vec() } else { frame::resize_bilinear(gray, w, h, sw, sh) };
                let ii = integral(&small, sw, sh);
                let stride = sw + 1;
                let step = if factor > 2.0 { 1 } else { 2 };
                let mut y = 0;
                while y + self.win_h <= sh {
                    let mut x = 0;
                    while x + self.win_w <= sw {
                        if self.eval(&ii, stride, x, y) {
                            raw.push([x as f32 * factor, y as f32 * factor, ww, wh]);
                        }
                        x += step;
                    }
                    y += step;
                }
            }
            factor *= scale_factor.max(1.05);
        }
        group(&raw, min_neighbors, 0.2)
    }
}

fn integral(img: &[u8], w: usize, h: usize) -> Vec<i32> {
    let stride = w + 1;
    let mut ii = vec![0i32; stride * (h + 1)];
    for y in 0..h {
        let mut row = 0i32;
        for x in 0..w {
            row += img[y * w + x] as i32;
            ii[(y + 1) * stride + x + 1] = ii[y * stride + x + 1] + row;
        }
    }
    ii
}

fn similar(a: &[f32; 4], b: &[f32; 4], eps: f32) -> bool {
    let d = eps * (a[2].min(b[2]) + a[3].min(b[3])) * 0.5;
    (a[0] - b[0]).abs() <= d
        && (a[1] - b[1]).abs() <= d
        && (a[0] + a[2] - b[0] - b[2]).abs() <= d
        && (a[1] + a[3] - b[1] - b[3]).abs() <= d
}

/// OpenCV's groupRectangles: cluster overlapping hits, average each cluster,
/// keep clusters with enough votes and drop small boxes inside bigger ones.
fn group(rects: &[[f32; 4]], min_neighbors: u32, eps: f32) -> Vec<FaceRect> {
    let n = rects.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut Vec<usize>, mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if similar(&rects[i], &rects[j], eps) {
                let ri = find(&mut parent, i);
                let rj = find(&mut parent, j);
                if ri != rj {
                    parent[rj] = ri;
                }
            }
        }
    }
    let mut sums: Vec<([f32; 4], u32)> = Vec::new();
    let mut root_index: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        let r = find(&mut parent, i);
        let k = match root_index[r] {
            Some(k) => k,
            None => {
                sums.push(([0.0; 4], 0));
                root_index[r] = Some(sums.len() - 1);
                sums.len() - 1
            }
        };
        for t in 0..4 {
            sums[k].0[t] += rects[i][t];
        }
        sums[k].1 += 1;
    }
    let avg: Vec<([f32; 4], u32)> = sums
        .iter()
        .map(|(s, c)| ([s[0] / *c as f32, s[1] / *c as f32, s[2] / *c as f32, s[3] / *c as f32], *c))
        .collect();
    let mut out = Vec::new();
    for (i, (r1, n1)) in avg.iter().enumerate() {
        if *n1 <= min_neighbors {
            continue;
        }
        let mut inside = false;
        for (j, (r2, n2)) in avg.iter().enumerate() {
            if i == j || *n2 <= min_neighbors {
                continue;
            }
            let dx = (r2[2] * eps).floor();
            let dy = (r2[3] * eps).floor();
            if r1[0] >= r2[0] - dx
                && r1[1] >= r2[1] - dy
                && r1[0] + r1[2] <= r2[0] + r2[2] + dx
                && r1[1] + r1[3] <= r2[1] + r2[3] + dy
                && (*n2 > (*n1).max(3) || *n1 < 3)
            {
                inside = true;
                break;
            }
        }
        if !inside {
            out.push(FaceRect { x: r1[0], y: r1[1], w: r1[2], h: r1[3], votes: *n1 });
        }
    }
    out
}

fn uniform_map() -> &'static [u8; 256] {
    static MAP: OnceLock<[u8; 256]> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = [58u8; 256];
        let mut k = 0u8;
        for c in 0..256u32 {
            let mut t = 0;
            for i in 0..8 {
                if ((c >> i) & 1) != ((c >> ((i + 1) % 8)) & 1) {
                    t += 1;
                }
            }
            if t <= 2 {
                m[c as usize] = k;
                k += 1;
            }
        }
        m
    })
}

/// Center-weighted 7x7 LBPH grid weights (Ahonen et al.): emphasize eyes,
/// eyebrows, nose and mouth while down-weighting outer background corners.
const CELL_WEIGHTS: [f32; GRID * GRID] = [
    0.3, 0.6, 0.8, 0.8, 0.8, 0.6, 0.3,
    0.5, 1.3, 1.8, 1.4, 1.8, 1.3, 0.5,
    0.5, 1.5, 2.0, 1.6, 2.0, 1.5, 0.5,
    0.4, 1.0, 1.4, 1.6, 1.4, 1.0, 0.4,
    0.4, 0.9, 1.2, 1.3, 1.2, 0.9, 0.4,
    0.3, 0.7, 1.3, 1.5, 1.3, 0.7, 0.3,
    0.2, 0.4, 0.7, 0.8, 0.7, 0.4, 0.2,
];

/// Mild clipped histogram equalization on a 64x64 face crop so changing room
/// lighting and webcam auto-exposure don't throw off LBP textures.
fn equalize_face(crop: &mut [u8]) {
    let n = crop.len();
    if n == 0 {
        return;
    }
    let mut hist = [0u32; 256];
    for &v in crop.iter() {
        hist[v as usize] += 1;
    }
    // Clip histogram peaks (CLAHE-style limit) to avoid amplifying flat noise.
    let clip = ((n / 256) * 4).max(16) as u32;
    let mut excess = 0u32;
    for h in hist.iter_mut() {
        if *h > clip {
            excess += *h - clip;
            *h = clip;
        }
    }
    let add = excess / 256;
    let rem = (excess % 256) as usize;
    for (i, h) in hist.iter_mut().enumerate() {
        *h += add + u32::from(i < rem);
    }
    let mut cdf = [0u32; 256];
    let mut acc = 0u32;
    let mut cdf_min = 0u32;
    for i in 0..256 {
        acc += hist[i];
        cdf[i] = acc;
        if cdf_min == 0 && acc > 0 {
            cdf_min = acc;
        }
    }
    let denom = (n as u32).saturating_sub(cdf_min).max(1) as f32;
    for v in crop.iter_mut() {
        let eq = (((cdf[*v as usize].saturating_sub(cdf_min)) as f32 / denom) * 255.0).clamp(0.0, 255.0);
        // Blend 70% equalized + 30% original to keep natural shading cues.
        *v = (eq * 0.70 + *v as f32 * 0.30).round().clamp(0.0, 255.0) as u8;
    }
}

/// The LBP histogram fingerprint of a face, quantized to bytes.
pub fn describe(gray: &[u8], w: usize, h: usize, r: &FaceRect) -> Option<Vec<u8>> {
    if r.w < 20.0 || r.h < 20.0 {
        return None;
    }
    let mut crop = frame::crop_resize(gray, w, h, r.x, r.y, r.w, r.h, FACE, FACE);
    equalize_face(&mut crop);
    let um = uniform_map();
    let s = FACE - 2;
    let mut codes = vec![0u8; s * s];
    for y in 1..FACE - 1 {
        for x in 1..FACE - 1 {
            let c = crop[y * FACE + x];
            let nb = [
                crop[(y - 1) * FACE + x - 1],
                crop[(y - 1) * FACE + x],
                crop[(y - 1) * FACE + x + 1],
                crop[y * FACE + x + 1],
                crop[(y + 1) * FACE + x + 1],
                crop[(y + 1) * FACE + x],
                crop[(y + 1) * FACE + x - 1],
                crop[y * FACE + x - 1],
            ];
            let mut code = 0u32;
            for (i, &v) in nb.iter().enumerate() {
                if v >= c {
                    code |= 1u32 << (7 - i);
                }
            }
            codes[(y - 1) * s + (x - 1)] = um[code as usize];
        }
    }
    let mut out = vec![0u8; DIM];
    for gy in 0..GRID {
        for gx in 0..GRID {
            let (y0, y1) = (gy * s / GRID, (gy + 1) * s / GRID);
            let (x0, x1) = (gx * s / GRID, (gx + 1) * s / GRID);
            let mut hist = [0u32; BINS];
            let mut n = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    hist[codes[y * s + x] as usize] += 1;
                    n += 1;
                }
            }
            let base = (gy * GRID + gx) * BINS;
            for b in 0..BINS {
                out[base + b] = ((hist[b] as f32 / n.max(1) as f32) * 255.0).round().min(255.0) as u8;
            }
        }
    }
    Some(out)
}

pub fn chi2(a: &[u8], b: &[u8]) -> f32 {
    let mut s = 0.0f32;
    let cells = (a.len().min(b.len())) / BINS;
    for cell in 0..cells.min(CELL_WEIGHTS.len()) {
        let w = CELL_WEIGHTS[cell];
        let base = cell * BINS;
        let mut cs = 0.0f32;
        for k in 0..BINS {
            let xa = a[base + k] as f32 / 255.0;
            let ya = b[base + k] as f32 / 255.0;
            let t = xa + ya;
            if t > 1e-6 {
                let d = xa - ya;
                cs += d * d / t;
            }
        }
        s += cs * w;
    }
    s
}

/// Works out how close a face must be to count as this person, from how
/// much their own enrollment photos differ from each other.
pub fn threshold_for(samples: &[Vec<u8>]) -> f32 {
    let n = samples.len();
    if n < 4 {
        return 30.0;
    }
    let mut d = Vec::new();
    for i in 0..n {
        let mut best = f32::MAX;
        for j in 0..n {
            if (i as i64 - j as i64).abs() < 3 {
                continue;
            }
            best = best.min(chi2(&samples[i], &samples[j]));
        }
        if best < f32::MAX {
            d.push(best);
        }
    }
    if d.is_empty() {
        return 30.0;
    }
    d.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = d[d.len() / 2];
    let p90 = d[(d.len() * 9 / 10).min(d.len() - 1)];
    (med * 1.48).max(p90 * 1.18).clamp(med * 1.25, med * 2.1).max(14.0)
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Person {
    pub name: String,
    pub samples: Vec<String>,
    pub threshold: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FaceDb {
    pub people: Vec<Person>,
}

/// The face database decoded and ready for matching.
#[derive(Default)]
pub struct Gallery {
    pub people: Vec<(String, Vec<Vec<u8>>, f32)>,
}

impl FaceDb {
    pub fn path() -> PathBuf {
        util::data_dir().join("faces.json")
    }

    pub fn load() -> FaceDb {
        util::load_json::<FaceDb>(&Self::path()).unwrap_or_default()
    }

    pub fn save(&self) {
        util::save_json(&Self::path(), self);
    }

    pub fn gallery(&self) -> Gallery {
        let people = self
            .people
            .iter()
            .map(|p| {
                let samples: Vec<Vec<u8>> = p
                    .samples
                    .iter()
                    .filter_map(|s| util::hex_decode(s))
                    .filter(|v| v.len() == DIM)
                    .collect();
                let thr = if p.threshold > 0.0 { p.threshold } else { threshold_for(&samples) };
                (p.name.clone(), samples, thr)
            })
            .collect();
        Gallery { people }
    }

    pub fn set_person(&mut self, name: &str, samples: &[Vec<u8>]) {
        let threshold = threshold_for(samples);
        let encoded: Vec<String> = samples.iter().map(|s| util::hex_encode(s)).collect();
        if let Some(p) = self.people.iter_mut().find(|p| p.name == name) {
            p.samples = encoded;
            p.threshold = threshold;
        } else {
            self.people.push(Person { name: name.to_string(), samples: encoded, threshold });
        }
    }

    /// Adds one more confirmed sample, keeping the original enrollment and
    /// the most recent learned faces.
    pub fn add_sample(&mut self, name: &str, sample: &[u8]) {
        if let Some(p) = self.people.iter_mut().find(|p| p.name == name) {
            p.samples.push(util::hex_encode(sample));
            if p.samples.len() > 60 {
                p.samples.remove(24);
            }
            let decoded: Vec<Vec<u8>> = p.samples.iter().filter_map(|s| util::hex_decode(s)).collect();
            p.threshold = threshold_for(&decoded);
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.people.retain(|p| p.name != name);
    }
}

impl Gallery {
    /// Closest person and how close they are, as distance divided by that
    /// person's threshold (1.0 or less means it's them).
    pub fn best(&self, d: &[u8], strictness: u8) -> Option<(usize, f32)> {
        let scale = match strictness {
            1 => 1.12,
            3 => 0.88,
            _ => 1.0,
        };
        let mut best: Option<(usize, f32)> = None;
        for (i, (_, samples, thr)) in self.people.iter().enumerate() {
            let mut dmin = f32::MAX;
            for s in samples {
                dmin = dmin.min(chi2(d, s));
            }
            if dmin == f32::MAX {
                continue;
            }
            let ratio = dmin / (thr * scale).max(1.0);
            if best.map_or(true, |(_, r)| ratio < r) {
                best = Some((i, ratio));
            }
        }
        best
    }
}
