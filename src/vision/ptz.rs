//! Pan and tilt control for USB PTZ webcams (through the standard UVC
//! controls) and OBSBOT cameras (through OBSBOT Center's OSC remote).
//!
//! Following leads the target: it aims where the person will be by the time
//! the motor gets there, and leads further the faster they move. When they
//! get away, it builds a search plan: first where they were heading, then the
//! doorways and paths the room map says people usually take from there, then
//! a sweep that starts on the side they ran toward, then home.

use std::collections::VecDeque;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

use nokhwa::utils::{ControlValueSetter, KnownCameraControl};
use serde::{Deserialize, Serialize};

use super::frame;
use crate::config::{NRect, PtzCfg, PtzKind};

/// Converts between the current view, scene angles (degrees) and map space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xf {
    pub active: bool,
    pub pan: f32,
    pub tilt: f32,
    pub gain_pan: f32,
    pub gain_tilt: f32,
    pub hfov: f32,
    pub vfov: f32,
    pub az_min: f32,
    pub az_max: f32,
    pub el_min: f32,
    pub el_max: f32,
}

fn nonzero(g: f32) -> f32 {
    if g.abs() < 0.05 {
        if g < 0.0 {
            -0.05
        } else {
            0.05
        }
    } else {
        g
    }
}

impl Xf {
    pub fn view_to_scene(&self, x: f32, y: f32) -> (f32, f32) {
        (self.pan * self.gain_pan + (x - 0.5) * self.hfov, self.tilt * self.gain_tilt - (y - 0.5) * self.vfov)
    }
    pub fn scene_to_view(&self, az: f32, el: f32) -> (f32, f32) {
        ((az - self.pan * self.gain_pan) / self.hfov + 0.5, 0.5 - (el - self.tilt * self.gain_tilt) / self.vfov)
    }
    pub fn scene_to_map(&self, az: f32, el: f32) -> (f32, f32) {
        (
            (az - self.az_min) / (self.az_max - self.az_min).max(1.0),
            (self.el_max - el) / (self.el_max - self.el_min).max(1.0),
        )
    }
    pub fn map_to_scene(&self, u: f32, v: f32) -> (f32, f32) {
        (self.az_min + u * (self.az_max - self.az_min), self.el_max - v * (self.el_max - self.el_min))
    }
    pub fn view_to_map(&self, x: f32, y: f32) -> (f32, f32) {
        if !self.active {
            return (x, y);
        }
        let (a, e) = self.view_to_scene(x, y);
        self.scene_to_map(a, e)
    }
    pub fn map_to_view(&self, u: f32, v: f32) -> (f32, f32) {
        if !self.active {
            return (u, v);
        }
        let (a, e) = self.map_to_scene(u, v);
        self.scene_to_view(a, e)
    }
    pub fn rect_map_to_view(&self, r: NRect) -> NRect {
        let (x0, y0) = self.map_to_view(r.x, r.y);
        let (x1, y1) = self.map_to_view(r.x + r.w, r.y + r.h);
        NRect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
    }
    pub fn rect_view_to_map(&self, r: NRect) -> NRect {
        let (x0, y0) = self.view_to_map(r.x, r.y);
        let (x1, y1) = self.view_to_map(r.x + r.w, r.y + r.h);
        NRect::new(x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs())
    }
    /// How big one view is in map units.
    pub fn view_scale(&self) -> (f32, f32) {
        if !self.active {
            return (1.0, 1.0);
        }
        (
            self.hfov / (self.az_max - self.az_min).max(1.0),
            self.vfov / (self.el_max - self.el_min).max(1.0),
        )
    }
    /// The pan and tilt command that centres a scene direction.
    pub fn command_for(&self, az: f32, el: f32) -> (f32, f32) {
        (az / nonzero(self.gain_pan), el / nonzero(self.gain_tilt))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Idle,
    Following,
    Searching,
    Returning,
    Mapping,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Idle => "Watching",
            Mode::Following => "Following",
            Mode::Searching => "Searching",
            Mode::Returning => "Returning home",
            Mode::Mapping => "Mapping the room",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pub az: f32,
    pub el: f32,
    pub vaz: f32,
    pub vel: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub az: f32,
    pub el: f32,
    pub dwell: f32,
    pub why: &'static str,
}

/// Calibration that's saved per camera.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PtzSave {
    pub calibrated: bool,
    pub gain_pan: f32,
    pub gain_tilt: f32,
    pub az_min: f32,
    pub az_max: f32,
    pub el_min: f32,
    pub el_max: f32,
}

pub struct Ptz {
    pub cfg: PtzCfg,
    pub hfov: f32,
    pub vfov: f32,
    pub pan: f32,
    pub tilt: f32,
    pub pan_lim: (f32, f32),
    pub tilt_lim: (f32, f32),
    pub gain_pan: f32,
    pub gain_tilt: f32,
    pub calibrated: bool,
    pub az_min: f32,
    pub az_max: f32,
    pub el_min: f32,
    pub el_max: f32,
    pub mode: Mode,
    pub plan: VecDeque<Look>,
    look_until: Option<Instant>,
    pub moving_until: Option<Instant>,
    last_cmd: Option<Instant>,
    last_target: Option<Target>,
    search_deadline: Option<Instant>,
    sock: Option<UdpSocket>,
    obsbot_ready: bool,
    pub status: String,
    pub why: &'static str,
    pub errors: u32,
}

impl Ptz {
    pub fn new(cfg: &PtzCfg, hfov: f32) -> Ptz {
        let mut p = Ptz {
            cfg: cfg.clone(),
            hfov,
            vfov: hfov * 0.6,
            pan: cfg.home_pan,
            tilt: cfg.home_tilt,
            pan_lim: (-129.0, 129.0),
            tilt_lim: (-59.0, 59.0),
            gain_pan: 1.0,
            gain_tilt: 1.0,
            calibrated: false,
            az_min: 0.0,
            az_max: 0.0,
            el_min: 0.0,
            el_max: 0.0,
            mode: Mode::Idle,
            plan: VecDeque::new(),
            look_until: None,
            moving_until: None,
            last_cmd: None,
            last_target: None,
            search_deadline: None,
            sock: None,
            obsbot_ready: false,
            status: String::new(),
            why: "Watching",
            errors: 0,
        };
        p.reset_extent();
        p
    }

    pub fn active(&self) -> bool {
        self.cfg.kind != PtzKind::Fixed
    }

    pub fn reset_extent(&mut self) {
        let a = self.pan_lim.0 * self.gain_pan;
        let b = self.pan_lim.1 * self.gain_pan;
        self.az_min = a.min(b) - self.hfov * 0.5;
        self.az_max = a.max(b) + self.hfov * 0.5;
        let c = self.tilt_lim.0 * self.gain_tilt;
        let d = self.tilt_lim.1 * self.gain_tilt;
        self.el_min = c.min(d) - self.vfov * 0.5;
        self.el_max = c.max(d) + self.vfov * 0.5;
    }

    pub fn save_state(&self) -> PtzSave {
        PtzSave {
            calibrated: self.calibrated,
            gain_pan: self.gain_pan,
            gain_tilt: self.gain_tilt,
            az_min: self.az_min,
            az_max: self.az_max,
            el_min: self.el_min,
            el_max: self.el_max,
        }
    }

    pub fn load_state(&mut self, s: &PtzSave) {
        if s.calibrated && s.gain_pan.abs() > 0.05 && s.az_max > s.az_min && s.el_max > s.el_min {
            self.calibrated = true;
            self.gain_pan = s.gain_pan;
            self.gain_tilt = if s.gain_tilt.abs() > 0.05 { s.gain_tilt } else { 1.0 };
            self.az_min = s.az_min;
            self.az_max = s.az_max;
            self.el_min = s.el_min;
            self.el_max = s.el_max;
        }
    }

    pub fn xf(&self) -> Xf {
        Xf {
            active: self.active(),
            pan: self.pan,
            tilt: self.tilt,
            gain_pan: self.gain_pan,
            gain_tilt: self.gain_tilt,
            hfov: self.hfov,
            vfov: self.vfov,
            az_min: self.az_min,
            az_max: self.az_max,
            el_min: self.el_min,
            el_max: self.el_max,
        }
    }

    pub fn is_moving(&self, now: Instant) -> bool {
        self.moving_until.map_or(false, |t| now < t)
    }

    fn ready_for_cmd(&self, now: Instant) -> bool {
        let gap = match self.cfg.kind {
            PtzKind::Uvc => 0.25,
            _ => 0.12,
        };
        self.last_cmd
            .map_or(true, |t| now.saturating_duration_since(t).as_secs_f32() >= gap)
    }

    /// Points the camera. Pan and tilt are in degrees.
    pub fn send(&mut self, cam: Option<&mut nokhwa::Camera>, pan: f32, tilt: f32, speed: u8, now: Instant) -> bool {
        let pan = pan.clamp(self.pan_lim.0, self.pan_lim.1);
        let tilt = tilt.clamp(self.tilt_lim.0, self.tilt_lim.1);
        let pan_out = if self.cfg.invert_pan { -pan } else { pan };
        let tilt_out = if self.cfg.invert_tilt { -tilt } else { tilt };
        let ok = match self.cfg.kind {
            PtzKind::Fixed => false,
            PtzKind::Uvc => match cam {
                Some(c) => {
                    let a = c.set_camera_control(
                        KnownCameraControl::Pan,
                        ControlValueSetter::Integer((pan_out * 3600.0) as i64),
                    );
                    let b = c.set_camera_control(
                        KnownCameraControl::Tilt,
                        ControlValueSetter::Integer((tilt_out * 3600.0) as i64),
                    );
                    if let Err(e) = &a {
                        self.status = format!("The camera refused a pan command: {}", e);
                    }
                    if let Err(e) = &b {
                        self.status = format!("The camera refused a tilt command: {}", e);
                    }
                    a.is_ok() || b.is_ok()
                }
                None => false,
            },
            PtzKind::Obsbot => {
                self.ensure_obsbot();
                let (p, t, s) = (pan_out.round() as i32, tilt_out.round() as i32, speed as i32);
                let args: Vec<i32> = if self.cfg.obsbot_four_args { vec![0, s, p, t] } else { vec![s, p, t] };
                self.osc("/OBSBOT/WebCam/General/SetGimMotorDegree", &args)
            }
        };
        if ok {
            let dist = (pan - self.pan).abs().max((tilt - self.tilt).abs());
            self.errors = 0;
            let travel = dist / 90.0 + 0.35;
            self.moving_until = Some(now + Duration::from_secs_f32(travel));
            self.pan = pan;
            self.tilt = tilt;
            self.last_cmd = Some(now);
        } else {
            self.errors += 1;
        }
        ok
    }

    fn ensure_obsbot(&mut self) {
        if self.sock.is_none() {
            match UdpSocket::bind("0.0.0.0:0") {
                Ok(s) => self.sock = Some(s),
                Err(e) => self.status = format!("Couldn't open a network socket: {}", e),
            }
        }
        if !self.obsbot_ready && self.sock.is_some() {
            self.obsbot_ready = true;
            // Same start up sequence as the original browser version.
            self.osc("/OBSBOT/WebCam/General/Connected", &[0]);
            self.osc("/OBSBOT/WebCam/Tiny/SetTrackingMode", &[0]);
            self.osc("/OBSBOT/WebCam/General/SetZoom", &[0]);
        }
    }

    fn osc(&mut self, addr: &str, args: &[i32]) -> bool {
        let target = format!("{}:{}", self.cfg.obsbot_host.trim(), self.cfg.obsbot_port);
        let msg = osc_message(addr, args);
        let result = match self.sock.as_ref() {
            Some(sock) => sock.send_to(&msg, target.as_str()).map(|_| ()),
            None => return false,
        };
        match result {
            Ok(()) => true,
            Err(e) => {
                self.status = format!("OBSBOT Center isn't reachable on {}: {}", target, e);
                false
            }
        }
    }

    pub fn go_home(&mut self, cam: Option<&mut nokhwa::Camera>, now: Instant) {
        let (p, t) = (self.cfg.home_pan, self.cfg.home_tilt);
        self.send(cam, p, t, 50, now);
        self.plan.clear();
        self.mode = Mode::Idle;
        self.why = "Watching from home";
    }

    /// One control step. `target` is the person being followed (scene
    /// degrees). `candidates` are places worth checking if they get away.
    pub fn follow(&mut self, cam: Option<&mut nokhwa::Camera>, target: Option<Target>, candidates: &[(f32, f32, &'static str)], now: Instant) {
        if !self.active() || !self.cfg.follow || self.mode == Mode::Mapping {
            return;
        }
        if let Some(t) = target {
            if self.mode != Mode::Following {
                self.plan.clear();
            }
            self.mode = Mode::Following;
            self.why = "Following";
            self.search_deadline = None;
            self.last_target = Some(t);
            let fast = (t.vaz * t.vaz + t.vel * t.vel).sqrt() > 35.0;
            let lead = if fast { 0.7 } else { 0.45 };
            let aim_az = t.az + t.vaz * lead;
            let aim_el = t.el + t.vel * lead * 0.5;
            let dz = aim_az - self.pan * self.gain_pan;
            let de = aim_el - self.tilt * self.gain_tilt;
            if dz.abs() < self.hfov * 0.12 && de.abs() < self.vfov * 0.14 {
                return;
            }
            if !self.ready_for_cmd(now) {
                return;
            }
            let (cp, ct) = self.xf().command_for(aim_az, aim_el);
            let speed = ((dz.abs().max(de.abs()) * 2.0) as u32).clamp(25, 100) as u8;
            self.send(cam, cp, ct, speed, now);
            return;
        }
        match self.mode {
            Mode::Following => {
                if let Some(t) = self.last_target.take() {
                    self.plan_search(t, candidates, now);
                }
                self.mode = Mode::Searching;
            }
            Mode::Searching | Mode::Returning => self.run_plan(cam, now),
            _ => {}
        }
    }

    fn plan_search(&mut self, t: Target, candidates: &[(f32, f32, &'static str)], now: Instant) {
        self.plan.clear();
        let tau = 0.8f32;
        for dt in [0.7f32, 1.6] {
            let k = tau * (1.0 - (-dt / tau).exp());
            self.plan.push_back(Look {
                az: t.az + t.vaz * k,
                el: t.el + t.vel * k * 0.5,
                dwell: 0.8,
                why: "Checking where they were heading",
            });
        }
        for c in candidates.iter().take(3) {
            self.plan.push_back(Look { az: c.0, el: c.1, dwell: 1.1, why: c.2 });
        }
        let dir = if t.vaz >= 0.0 { 1.0 } else { -1.0 };
        let step = self.hfov * 0.8;
        let lo = self.az_min + self.hfov * 0.5;
        let hi = self.az_max - self.hfov * 0.5;
        let el = self.cfg.home_tilt * self.gain_tilt;
        for pass in 0..2 {
            let d = if pass == 0 { dir } else { -dir };
            let mut a = t.az;
            loop {
                a += d * step;
                if a < lo - 1.0 || a > hi + 1.0 || self.plan.len() > 16 {
                    break;
                }
                self.plan.push_back(Look { az: a.clamp(lo, hi), el, dwell: 0.9, why: "Sweeping the room" });
            }
        }
        self.plan.push_back(Look {
            az: self.cfg.home_pan * self.gain_pan,
            el: self.cfg.home_tilt * self.gain_tilt,
            dwell: 0.0,
            why: "Back to its home view",
        });
        self.look_until = None;
        self.search_deadline = Some(now + Duration::from_secs_f32(self.cfg.search_secs.max(5.0)));
    }

    fn run_plan(&mut self, cam: Option<&mut nokhwa::Camera>, now: Instant) {
        if self.search_deadline.map_or(false, |d| now > d) {
            self.search_deadline = None;
            self.go_home(cam, now);
            return;
        }
        if let Some(until) = self.look_until {
            if now < until {
                return;
            }
            self.look_until = None;
            self.plan.pop_front();
        }
        let next = match self.plan.front() {
            Some(l) => *l,
            None => {
                self.mode = Mode::Idle;
                self.why = "Watching";
                return;
            }
        };
        if self.plan.len() == 1 {
            self.mode = Mode::Returning;
        }
        let (cp, ct) = self.xf().command_for(next.az, next.el);
        self.why = next.why;
        if self.send(cam, cp, ct, 60, now) {
            let travel = self
                .moving_until
                .map(|m| m.saturating_duration_since(now).as_secs_f32())
                .unwrap_or(0.0);
            self.look_until = Some(now + Duration::from_secs_f32(travel + next.dwell));
        } else {
            self.plan.pop_front();
        }
    }
}

pub fn osc_message(addr: &str, args: &[i32]) -> Vec<u8> {
    fn push_padded(b: &mut Vec<u8>, s: &[u8]) {
        b.extend_from_slice(s);
        b.push(0);
        while b.len() % 4 != 0 {
            b.push(0);
        }
    }
    let mut b = Vec::new();
    push_padded(&mut b, addr.as_bytes());
    let mut tags = String::from(",");
    for _ in args {
        tags.push('i');
    }
    push_padded(&mut b, tags.as_bytes());
    for a in args {
        b.extend_from_slice(&a.to_be_bytes());
    }
    b
}

/// Brightness changes across the image, column by column. Comparing two of
/// these tells how far the picture shifted between two camera positions.
pub fn profile(gray: &[u8], w: usize, h: usize) -> Vec<f32> {
    if w < 4 || h < 4 || gray.len() < w * h {
        return Vec::new();
    }
    let (y0, y1) = (h / 6, h - h / 6);
    let mut p = vec![0f32; w];
    for (x, slot) in p.iter_mut().enumerate() {
        let mut s = 0f32;
        for y in y0..y1 {
            s += gray[y * w + x] as f32;
        }
        *slot = s / (y1 - y0).max(1) as f32;
    }
    let mut d: Vec<f32> = (1..w).map(|x| p[x] - p[x - 1]).collect();
    let n = d.len() as f32;
    let mean = d.iter().sum::<f32>() / n;
    let std = (d.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n).sqrt().max(1e-3);
    for v in d.iter_mut() {
        *v = (*v - mean) / std;
    }
    d
}

/// The shift `s` (with sub pixel refinement) where `a[i + s]` best matches `b[i]`.
pub fn best_shift(a: &[f32], b: &[f32], max_shift: usize) -> Option<(f32, f32)> {
    let n = a.len().min(b.len());
    if n < 16 {
        return None;
    }
    let max_s = max_shift.min(n * 3 / 4) as i32;
    let mut scores = vec![f32::MIN; (2 * max_s + 1) as usize];
    let mut best = (0i32, f32::MIN);
    for s in -max_s..=max_s {
        let mut num = 0f32;
        let mut cnt = 0i32;
        for i in 0..n as i32 {
            let j = i + s;
            if j < 0 || j >= n as i32 {
                continue;
            }
            num += a[j as usize] * b[i as usize];
            cnt += 1;
        }
        if cnt < n as i32 / 3 {
            continue;
        }
        let sc = num / cnt as f32;
        scores[(s + max_s) as usize] = sc;
        if sc > best.1 {
            best = (s, sc);
        }
    }
    if best.1 == f32::MIN {
        return None;
    }
    let k = (best.0 + max_s) as usize;
    let mut sub = best.0 as f32;
    if k > 0 && k + 1 < scores.len() && scores[k - 1] > f32::MIN && scores[k + 1] > f32::MIN {
        let (l, c, r) = (scores[k - 1], scores[k], scores[k + 1]);
        let den = l - 2.0 * c + r;
        if den.abs() > 1e-6 {
            sub += 0.5 * (l - r) / den;
        }
    }
    Some((sub, best.1))
}

/// The "map the room" sweep: steps across the pan range, measures how far the
/// picture really moved at each step to calibrate the motor, and stitches the
/// frames into a panorama.
pub struct Sweep {
    steps: Vec<f32>,
    idx: usize,
    wait_until: Option<Instant>,
    prev: Option<(Vec<f32>, f32)>,
    ratios: Vec<f32>,
    tiles: Vec<(f32, frame::Rgb)>,
    cum: f32,
    tilt: f32,
}

impl Sweep {
    pub fn new(p: &mut Ptz) -> Sweep {
        let step = (p.hfov * 0.5).max(10.0);
        let mut steps = Vec::new();
        let mut a = p.pan_lim.0;
        while a <= p.pan_lim.1 + 0.01 {
            steps.push(a);
            a += step;
        }
        p.mode = Mode::Mapping;
        p.why = "Mapping the room";
        p.plan.clear();
        p.status.clear();
        Sweep { steps, idx: 0, wait_until: None, prev: None, ratios: Vec::new(), tiles: Vec::new(), cum: 0.0, tilt: p.cfg.home_tilt }
    }

    pub fn progress(&self) -> f32 {
        self.idx as f32 / self.steps.len().max(1) as f32
    }

    /// Returns true once every step has been visited.
    pub fn tick(&mut self, p: &mut Ptz, cam: Option<&mut nokhwa::Camera>, gray: &[u8], w: usize, h: usize, tile: &frame::Rgb, now: Instant) -> bool {
        if self.idx >= self.steps.len() {
            return true;
        }
        match self.wait_until {
            None => {
                let pan = self.steps[self.idx];
                p.send(cam, pan, self.tilt, 40, now);
                let settle = if self.idx == 0 { 2.0 } else { 0.9 };
                let travel = p
                    .moving_until
                    .map(|m| m.saturating_duration_since(now).as_secs_f32())
                    .unwrap_or(0.0);
                self.wait_until = Some(now + Duration::from_secs_f32(travel + settle));
                false
            }
            Some(t) if now < t => false,
            Some(_) => {
                let prof = profile(gray, w, h);
                let pan = self.steps[self.idx];
                if let Some((prev, prev_pan)) = &self.prev {
                    let commanded = pan - *prev_pan;
                    match best_shift(prev, &prof, w * 3 / 4) {
                        Some((shift, score)) if score > 0.25 => {
                            let measured = shift / w.max(1) as f32 * p.hfov;
                            if commanded.abs() > 0.5 {
                                self.ratios.push(measured / commanded);
                            }
                            self.cum += measured;
                        }
                        _ => self.cum += commanded,
                    }
                }
                self.tiles.push((self.cum, tile.clone()));
                self.prev = Some((prof, pan));
                self.idx += 1;
                self.wait_until = None;
                self.idx >= self.steps.len()
            }
        }
    }

    /// Applies the calibration and returns the stitched panorama.
    pub fn finish(&self, p: &mut Ptz) -> Option<frame::Rgb> {
        p.mode = Mode::Idle;
        let mut r = self.ratios.clone();
        r.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let gain = if r.len() >= 2 { r[r.len() / 2] } else { 0.0 };
        if gain.abs() < 0.2 || gain.abs() > 3.0 || self.tiles.is_empty() {
            p.status = "The picture didn't move while mapping, so following stays off. Check the PTZ settings.".to_string();
            p.calibrated = false;
            return None;
        }
        p.gain_pan = gain;
        p.gain_tilt = gain.abs();
        p.calibrated = true;
        let first_az = self.steps[0] * gain;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for (off, _) in &self.tiles {
            lo = lo.min(*off);
            hi = hi.max(*off);
        }
        p.az_min = first_az + lo - p.hfov * 0.5;
        p.az_max = first_az + hi + p.hfov * 0.5;
        let el = self.tilt * p.gain_tilt;
        p.el_min = el - p.vfov;
        p.el_max = el + p.vfov;
        p.status = format!("Mapped {:.0} degrees of the room", p.az_max - p.az_min);
        build_panorama(&self.tiles, p.hfov)
    }
}

fn build_panorama(tiles: &[(f32, frame::Rgb)], hfov: f32) -> Option<frame::Rgb> {
    let first = &tiles.first()?.1;
    let (tw, th) = (first.w, first.h);
    let px_per_deg = tw as f32 / hfov.max(1.0);
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for (off, _) in tiles {
        lo = lo.min(*off);
        hi = hi.max(*off);
    }
    let width = ((hi - lo) * px_per_deg).max(0.0) as usize + tw;
    let mut acc = vec![0f32; width * th * 3];
    let mut wsum = vec![0f32; width * th];
    for (off, t) in tiles {
        if t.w != tw || t.h != th {
            continue;
        }
        let x0 = ((off - lo) * px_per_deg) as usize;
        for y in 0..th {
            for x in 0..tw {
                let ox = x0 + x;
                if ox >= width {
                    continue;
                }
                let feather = 1.0 - ((x as f32 + 0.5) / tw as f32 - 0.5).abs() * 1.8;
                let wgt = feather.max(0.05);
                let si = (y * tw + x) * 3;
                let di = y * width + ox;
                for c in 0..3 {
                    acc[di * 3 + c] += t.data[si + c] as f32 * wgt;
                }
                wsum[di] += wgt;
            }
        }
    }
    let mut data = vec![0u8; width * th * 3];
    for i in 0..width * th {
        let wv = wsum[i].max(1e-3);
        for c in 0..3 {
            data[i * 3 + c] = (acc[i * 3 + c] / wv).clamp(0.0, 255.0) as u8;
        }
    }
    Some(frame::Rgb { w: width, h: th, data })
}
