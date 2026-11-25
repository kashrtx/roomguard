//! Humanoid body detection and 17-joint skeleton tracking, Kinect style.
//!
//! Pixel motion alone can't tell a person from a webcam's auto exposure,
//! autofocus breathing or a poster flapping in a draft. So RoomGuard only
//! believes there is a person when a neural network sees a human body AND a
//! second network finds a coherent human skeleton inside it (head,
//! shoulders, torso, hips, limbs). This works when the face is turned away
//! or out of view, because the torso and shoulders are enough.
//!
//! Stage 1: RTMDet-Nano person detector (320x320), on the whole frame plus
//! up to two zoomed looks where something moved or where a small, far away
//! person is being followed. The zoomed look is what finds someone across
//! the room: at full frame they're only a few dozen pixels tall.
//! Stage 2: RTMPose-Tiny top-down pose (256x192, 17 COCO joints, SimCC).
//! Both are Apache-2.0 models from OpenMMLab, run by `tract`, a pure Rust
//! inference engine, so nothing extra has to be installed on any platform.
//!
//! The thresholds below were measured, not guessed: real photos (close,
//! far, torso only, dark, blurred, noisy) against cats, horses, posters,
//! textures and clothes hanging in a real bedroom. Clothes are the hardest
//! case because a shirt on a hook is shaped like a torso, which is why the
//! final say belongs to the tracker: nothing becomes a person until it has
//! been seen clearly several times and has actually moved.
//!
//! Inference runs on its own thread so the camera and the UI never wait on it.

use std::io::Cursor;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::Instant;

use tract_onnx::prelude::*;

use super::frame::Rgb;
use crate::config::NRect;
use crate::util;

static DET_ONNX: &[u8] = include_bytes!("../../assets/models/person_det.onnx");
static POSE_ONNX: &[u8] = include_bytes!("../../assets/models/body_pose.onnx");

const DET: usize = 320;
const POSE_W: usize = 192;
const POSE_H: usize = 256;
/// Lowest detector score worth running the skeleton network on.
const DET_MIN: f32 = 0.30;
/// Joints at or above this are drawn on the overlay.
pub const KP_THR: f32 = 0.30;
/// Joints at or above this count as clearly seen when judging a body.
pub const KP_SURE: f32 = 0.45;
pub const JOINTS: usize = 17;
/// Most zoomed looks per round, on top of the whole frame.
pub const MAX_ZOOMS: usize = 2;
/// Most skeletons per round.
const MAX_BODIES: usize = 4;

/// COCO joints: 0 nose, 1/2 eyes, 3/4 ears, 5/6 shoulders, 7/8 elbows,
/// 9/10 wrists, 11/12 hips, 13/14 knees, 15/16 ankles (left/right).
pub const BONES: [(usize, usize); 16] = [
    (5, 6),
    (5, 7),
    (7, 9),
    (6, 8),
    (8, 10),
    (5, 11),
    (6, 12),
    (11, 12),
    (11, 13),
    (13, 15),
    (12, 14),
    (14, 16),
    (0, 1),
    (0, 2),
    (1, 3),
    (2, 4),
];

type Plan = SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

/// How sure we are that a detection is a real human body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Evidence {
    /// Not a body.
    None,
    /// Body-like but not enough on its own. It can keep someone already
    /// being followed in view for a moment, never start a new person.
    Weak,
    /// A body.
    Normal,
    /// Unmistakably a body.
    Strong,
}

/// One detected person, all coordinates normalized to the frame (0..1).
#[derive(Clone, Copy, Debug)]
pub struct Person {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
    pub score: f32,
    /// (x, y, confidence) for each of the 17 COCO joints.
    pub kp: [(f32, f32, f32); JOINTS],
}

impl Person {
    pub fn cx(&self) -> f32 {
        (self.x0 + self.x1) * 0.5
    }
    pub fn cy(&self) -> f32 {
        (self.y0 + self.y1) * 0.5
    }
    pub fn w(&self) -> f32 {
        (self.x1 - self.x0).max(0.0)
    }
    pub fn h(&self) -> f32 {
        (self.y1 - self.y0).max(0.0)
    }

    fn sure(&self, i: usize) -> bool {
        self.kp[i].2 >= KP_SURE
    }

    /// Where this person's face can be: the top half of the body box. Face
    /// detection only runs inside these, so posters, corkboards and patterned
    /// fabric can never be mistaken for a face.
    pub fn head_region(&self) -> NRect {
        let (w, h) = (self.w(), self.h());
        NRect::new(self.x0 - w * 0.12, self.y0 - h * 0.06, w * 1.24, h * 0.52)
    }

    /// Weighs the detector against the skeleton so each vouches for the
    /// other. Shoulders are what separate a person from a coat: in testing,
    /// no non-person with a detector score of 0.55 or more ever had a
    /// shoulder above 0.27, while real people, even blurred, dark or seen
    /// from the chest down, stayed above 0.40.
    pub fn evidence(&self, det_thr: f32) -> Evidence {
        let (w, h) = (self.w(), self.h());
        if h < 0.06 || w < 0.02 {
            return Evidence::None;
        }
        let aspect = h / w.max(1e-3);
        if !(0.3..=6.5).contains(&aspect) {
            return Evidence::None;
        }
        let sure: Vec<usize> = (0..JOINTS).filter(|&i| self.sure(i)).collect();
        let n_sure = sure.len();
        let shoulder = self.kp[5].2.max(self.kp[6].2);
        let mut conf = [0.0f32; JOINTS];
        for (slot, k) in conf.iter_mut().zip(self.kp.iter()) {
            *slot = k.2;
        }
        conf.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let top8 = conf[..8].iter().sum::<f32>() / 8.0;
        // Clearly seen joints belong on the body, not scattered around it.
        if n_sure > 0 {
            let (ex0, ey0, ex1, ey1) = (self.x0 - w * 0.25, self.y0 - h * 0.25, self.x1 + w * 0.25, self.y1 + h * 0.25);
            let inside = sure
                .iter()
                .filter(|&&i| {
                    let (x, y, _) = self.kp[i];
                    x >= ex0 && x <= ex1 && y >= ey0 && y <= ey1
                })
                .count();
            if (inside as f32) < 0.7 * n_sure as f32 {
                return Evidence::None;
            }
        }
        // A torso has length when shoulders and hips are both clearly seen.
        let shoulders = [5usize, 6].iter().filter(|&&i| self.sure(i)).count();
        let hips = [11usize, 12].iter().filter(|&&i| self.sure(i)).count();
        if shoulders > 0 && hips > 0 {
            let mid = |a: usize, b: usize| {
                let pts: Vec<(f32, f32)> = [a, b].iter().filter(|&&i| self.sure(i)).map(|&i| (self.kp[i].0, self.kp[i].1)).collect();
                let n = pts.len().max(1) as f32;
                (pts.iter().map(|p| p.0).sum::<f32>() / n, pts.iter().map(|p| p.1).sum::<f32>() / n)
            };
            let s = mid(5, 6);
            let hp = mid(11, 12);
            let torso = ((s.0 - hp.0).powi(2) + (s.1 - hp.1).powi(2)).sqrt();
            if torso < h * 0.10 {
                return Evidence::None;
            }
        }
        if self.score >= 0.55 && shoulder >= 0.40 && n_sure >= 3 && top8 >= 0.44 {
            Evidence::Strong
        } else if self.score >= det_thr && shoulder >= 0.52 && n_sure >= 5 && top8 >= 0.50 {
            Evidence::Normal
        } else if self.score >= DET_MIN && shoulder >= 0.45 && n_sure >= 4 && top8 >= 0.46 {
            Evidence::Weak
        } else {
            Evidence::None
        }
    }

    /// True for a real human body: a sure detection with a coherent skeleton.
    /// The camera uses `evidence` directly; the tests use this.
    #[allow(dead_code)]
    pub fn humanoid(&self, det_thr: f32) -> bool {
        self.evidence(det_thr) >= Evidence::Normal
    }
}

pub struct PoseNet {
    det: Plan,
    pose: Plan,
}

impl PoseNet {
    pub fn load() -> TractResult<PoseNet> {
        // The detector's exported graph ends in NMS/TopK post-processing that
        // tract can't type. Cut it at the raw score and box heads and do the
        // decoding and NMS ourselves below.
        let mut raw = tract_onnx::onnx().model_for_read(&mut Cursor::new(DET_ONNX))?;
        raw.set_output_names(&["Sigmoid_533", "Concat_534"])?;
        raw = raw.eliminate_dead_branches()?;
        raw.set_input_fact(0, InferenceFact::dt_shape(f32::datum_type(), tvec!(1, 3, DET, DET)))?;
        let det = raw.into_optimized()?.into_runnable()?;

        // The pose graph has a symbolic batch dimension: type it symbolically,
        // then pin batch = 1.
        let mut raw = tract_onnx::onnx().model_for_read(&mut Cursor::new(POSE_ONNX))?;
        let batch = raw.symbols.sym("batch");
        raw.set_input_fact(
            0,
            InferenceFact::dt_shape(f32::datum_type(), tvec!(batch.to_dim(), 3.to_dim(), POSE_H.to_dim(), POSE_W.to_dim())),
        )?;
        let typed = raw.into_typed()?;
        let pose = typed
            .concretize_dims(&SymbolValues::default().with(&batch, 1))?
            .into_optimized()?
            .into_runnable()?;
        Ok(PoseNet { det, pose })
    }

    /// Every person candidate in the whole frame with its skeleton. Use
    /// `Person::evidence` to decide which ones are real.
    #[allow(dead_code)]
    pub fn detect(&self, img: &Rgb) -> Vec<Person> {
        self.detect_with(img, &[])
    }

    /// The whole frame plus a closer look inside each zoom region
    /// (normalized), so small, far away people are found too.
    pub fn detect_with(&self, img: &Rgb, zooms: &[NRect]) -> Vec<Person> {
        if img.w < 16 || img.h < 16 || img.data.len() < img.w * img.h * 3 {
            return Vec::new();
        }
        let (w, h) = (img.w as f32, img.h as f32);
        let mut hits: Vec<(f32, [f32; 4])> = match self.boxes_in(img, 0.0, 0.0, w, h) {
            Ok(b) => b,
            Err(e) => {
                util::log(&format!("body detector failed: {}", e));
                return Vec::new();
            }
        };
        for z in zooms.iter().take(MAX_ZOOMS) {
            let (rx, ry, rw, rh) = (z.x * w, z.y * h, z.w * w, z.h * h);
            if rw < 24.0 || rh < 24.0 {
                continue;
            }
            match self.boxes_in(img, rx, ry, rw, rh) {
                Ok(b) => hits.extend(b),
                Err(e) => util::log(&format!("body detector failed on a close look: {}", e)),
            }
        }
        let mut out = Vec::new();
        for (score, b) in suppress(hits, MAX_BODIES) {
            match self.skeleton(img, b) {
                Ok(kp) => out.push(Person { x0: b[0], y0: b[1], x1: b[2], y1: b[3], score, kp }),
                Err(e) => util::log(&format!("skeleton network failed: {}", e)),
            }
        }
        out
    }

    /// Person boxes inside a region of the image given in pixels. The region
    /// is letterboxed into the detector's square input (outside the picture
    /// reads as the grey RTMDet was trained with), and the boxes come back
    /// normalized to the whole image.
    fn boxes_in(&self, img: &Rgb, rx: f32, ry: f32, rw: f32, rh: f32) -> TractResult<Vec<(f32, [f32; 4])>> {
        let (w, h) = (img.w as f32, img.h as f32);
        let scale = (DET as f32 / rw.max(1.0)).min(DET as f32 / rh.max(1.0));
        let nw = ((rw * scale).round() as usize).min(DET);
        let nh = ((rh * scale).round() as usize).min(DET);
        // RTMDet expects BGR, mean/std normalized, letterboxed with 114 grey.
        let mean = [103.53f32, 116.28, 123.675];
        let std = [57.375f32, 57.12, 58.395];
        let mut a = tract_ndarray::Array4::<f32>::zeros((1, 3, DET, DET));
        for y in 0..DET {
            for x in 0..DET {
                let px = if x < nw && y < nh {
                    sample(img, rx + (x as f32 + 0.5) / scale - 0.5, ry + (y as f32 + 0.5) / scale - 0.5).unwrap_or([114.0; 3])
                } else {
                    [114.0; 3]
                };
                let bgr = [px[2], px[1], px[0]];
                for c in 0..3 {
                    a[[0, c, y, x]] = (bgr[c] - mean[c]) / std[c];
                }
            }
        }
        let outs = self.det.run(tvec!(a.into_tensor().into()))?;
        let scores = outs[0].to_array_view::<f32>()?;
        let regs = outs[1].to_array_view::<f32>()?;
        let n = scores.shape().get(1).copied().unwrap_or(0);
        if n != 2100 || regs.shape().get(1).copied().unwrap_or(0) != 2100 {
            return Ok(Vec::new());
        }
        let mut hits: Vec<(f32, [f32; 4])> = Vec::new();
        for &(grid, stride, offset) in &[(40usize, 8.0f32, 0usize), (20, 16.0, 1600), (10, 32.0, 2000)] {
            for gy in 0..grid {
                for gx in 0..grid {
                    let i = offset + gy * grid + gx;
                    let s = scores[[0, i, 0]];
                    if !(s >= DET_MIN) {
                        continue;
                    }
                    // Box head outputs left/top/right/bottom distances in input pixels.
                    let (cx, cy) = (gx as f32 * stride, gy as f32 * stride);
                    let x0 = ((rx + (cx - regs[[0, i, 0]]) / scale) / w).clamp(0.0, 1.0);
                    let y0 = ((ry + (cy - regs[[0, i, 1]]) / scale) / h).clamp(0.0, 1.0);
                    let x1 = ((rx + (cx + regs[[0, i, 2]]) / scale) / w).clamp(0.0, 1.0);
                    let y1 = ((ry + (cy + regs[[0, i, 3]]) / scale) / h).clamp(0.0, 1.0);
                    if x1 - x0 > 0.01 && y1 - y0 > 0.02 {
                        hits.push((s, [x0, y0, x1, y1]));
                    }
                }
            }
        }
        Ok(suppress(hits, 6))
    }

    fn skeleton(&self, img: &Rgb, b: [f32; 4]) -> TractResult<[(f32, f32, f32); JOINTS]> {
        let (w, h) = (img.w as f32, img.h as f32);
        // Standard top-down crop: box centre, 1.25x padding, fixed 3:4 aspect.
        let cx = (b[0] + b[2]) * 0.5 * w;
        let cy = (b[1] + b[3]) * 0.5 * h;
        let mut bw = (b[2] - b[0]) * w * 1.25;
        let mut bh = (b[3] - b[1]) * h * 1.25;
        let aspect = POSE_W as f32 / POSE_H as f32;
        if bw > bh * aspect {
            bh = bw / aspect;
        } else {
            bw = bh * aspect;
        }
        let (ox, oy) = (cx - bw * 0.5, cy - bh * 0.5);
        let mean = [123.675f32, 116.28, 103.53];
        let std = [58.395f32, 57.12, 57.375];
        let mut a = tract_ndarray::Array4::<f32>::zeros((1, 3, POSE_H, POSE_W));
        for py in 0..POSE_H {
            for px in 0..POSE_W {
                let sx = ox + (px as f32 + 0.5) / POSE_W as f32 * bw - 0.5;
                let sy = oy + (py as f32 + 0.5) / POSE_H as f32 * bh - 0.5;
                let rgb = sample(img, sx, sy).unwrap_or([0.0; 3]);
                for c in 0..3 {
                    a[[0, c, py, px]] = (rgb[c] - mean[c]) / std[c];
                }
            }
        }
        let outs = self.pose.run(tvec!(a.into_tensor().into()))?;
        let sx = outs[0].to_array_view::<f32>()?;
        let sy = outs[1].to_array_view::<f32>()?;
        let (nx, ny) = (sx.shape()[2], sy.shape()[2]);
        let mut kp = [(0.0f32, 0.0f32, 0.0f32); JOINTS];
        for (k, slot) in kp.iter_mut().enumerate().take(sx.shape()[1].min(JOINTS)) {
            let (mut bx, mut vx) = (0usize, f32::NEG_INFINITY);
            for i in 0..nx {
                let v = sx[[0, k, i]];
                if v > vx {
                    vx = v;
                    bx = i;
                }
            }
            let (mut by, mut vy) = (0usize, f32::NEG_INFINITY);
            for i in 0..ny {
                let v = sy[[0, k, i]];
                if v > vy {
                    vy = v;
                    by = i;
                }
            }
            // SimCC bins are split 2x finer than the input pixels.
            let ix = bx as f32 * POSE_W as f32 / nx as f32;
            let iy = by as f32 * POSE_H as f32 / ny as f32;
            let x = ((ox + ix / POSE_W as f32 * bw) / w).clamp(0.0, 1.0);
            let y = ((oy + iy / POSE_H as f32 * bh) / h).clamp(0.0, 1.0);
            let conf = vx.min(vy);
            *slot = (x, y, if conf.is_finite() { conf.max(0.0) } else { 0.0 });
        }
        Ok(kp)
    }
}

/// Bilinear RGB sample at a pixel position, None outside the image.
fn sample(img: &Rgb, fx: f32, fy: f32) -> Option<[f32; 3]> {
    let (w, h) = (img.w, img.h);
    if fx < -0.5 || fy < -0.5 || fx > w as f32 - 0.5 || fy > h as f32 - 0.5 {
        return None;
    }
    let x = fx.clamp(0.0, (w - 1) as f32);
    let y = fy.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (ax, ay) = (x - x0 as f32, y - y0 as f32);
    let d = &img.data;
    let mut out = [0.0f32; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let p = |xx: usize, yy: usize| d[(yy * w + xx) * 3 + c] as f32;
        let top = p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax;
        let bot = p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax;
        *o = top * (1.0 - ay) + bot * ay;
    }
    Some(out)
}

/// Best boxes first, dropping any that mostly repeat a better one. A box
/// mostly inside a better box is dropped too: a close look can cut a person
/// at its edge, and that half person is the same person.
fn suppress(mut hits: Vec<(f32, [f32; 4])>, max: usize) -> Vec<(f32, [f32; 4])> {
    hits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut keep: Vec<(f32, [f32; 4])> = Vec::new();
    for hit in hits {
        if !keep.iter().any(|k| iou(&k.1, &hit.1) > 0.45 || contained(&hit.1, &k.1) > 0.7) {
            keep.push(hit);
        }
        if keep.len() >= max {
            break;
        }
    }
    keep
}

/// How much of `a` lies inside `b`.
fn contained(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let area = (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0);
    iw * ih / (area + 1e-6)
}

fn iou(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let iw = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let ih = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = iw * ih;
    let aa = (a[2] - a[0]).max(0.0) * (a[3] - a[1]).max(0.0);
    let ab = (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0);
    inter / (aa + ab - inter + 1e-6)
}

pub struct PoseResult {
    pub people: Vec<Person>,
    pub ms: f32,
}

/// One frame for the body networks, with where to look closer.
pub struct PoseJob {
    pub img: Arc<Rgb>,
    pub zooms: Vec<NRect>,
}

pub const POSE_LOADING: u8 = 0;
pub const POSE_READY: u8 = 1;
pub const POSE_FAILED: u8 = 2;

/// Runs the body networks on a background thread. Frames are dropped while
/// it's busy, so it never builds a backlog or slows the camera down.
pub struct PoseWorker {
    tx: SyncSender<PoseJob>,
    pub rx: Receiver<PoseResult>,
    state: Arc<AtomicU8>,
}

impl PoseWorker {
    pub fn start() -> Option<PoseWorker> {
        let (tx, jobs) = mpsc::sync_channel::<PoseJob>(1);
        let (out, rx) = mpsc::channel();
        let state = Arc::new(AtomicU8::new(POSE_LOADING));
        let st = state.clone();
        let spawned = std::thread::Builder::new().name("body-tracking".into()).spawn(move || {
            let t0 = Instant::now();
            let net = match PoseNet::load() {
                Ok(n) => n,
                Err(e) => {
                    st.store(POSE_FAILED, Ordering::SeqCst);
                    util::log(&format!("body tracking models failed to load: {}", e));
                    return;
                }
            };
            st.store(POSE_READY, Ordering::SeqCst);
            util::log(&format!("body tracking ready in {} ms", t0.elapsed().as_millis()));
            while let Ok(job) = jobs.recv() {
                let t = Instant::now();
                let people = net.detect_with(&job.img, &job.zooms);
                let ms = t.elapsed().as_secs_f32() * 1000.0;
                if out.send(PoseResult { people, ms }).is_err() {
                    break;
                }
            }
        });
        match spawned {
            Ok(_) => Some(PoseWorker { tx, rx, state }),
            Err(e) => {
                util::log(&format!("could not start body tracking: {}", e));
                None
            }
        }
    }

    /// Hands over a frame. Returns false if the worker is still busy.
    pub fn submit(&self, job: PoseJob) -> bool {
        match self.tx.try_send(job) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }

    /// POSE_LOADING, POSE_READY or POSE_FAILED.
    pub fn state(&self) -> u8 {
        self.state.load(Ordering::SeqCst)
    }
}
