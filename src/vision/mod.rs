//! The camera thread. It owns the camera and runs the whole pipeline on every
//! frame: decode, motion, tracking, faces, the room map, the door watcher,
//! PTZ control and snapshots. The alarm panel only ever hears zone events
//! from here, the same way a real panel only hears its sensors.

pub mod demo;
pub mod faces;
pub mod frame;
pub mod map;
pub mod motion;
pub mod pose;
pub mod ptz;
pub mod track;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType};

use crate::config::{CameraCfg, NRect};
use crate::engine::EngineMsg;
use crate::util;

const FORMATS: [FrameFormat; 6] = [
    FrameFormat::MJPEG,
    FrameFormat::YUYV,
    FrameFormat::NV12,
    FrameFormat::RAWRGB,
    FrameFormat::RAWBGR,
    FrameFormat::GRAY,
];
const ENROLL_TARGET: usize = 24;
/// The device number of the built in demo camera.
pub const DEMO_DEVICE: u32 = 9999;

#[derive(Clone, Debug)]
pub enum VisionEvent {
    /// Zone 1 is the doorway (or the whole view), zone 2 the rest of the room.
    Motion { zone: u8, open: bool },
    Camera { online: bool, covered: bool },
    Recognized { name: String },
    Stranger,
    DoorOpen(bool),
    Snapshot { path: PathBuf, reason: String },
}

pub enum CamCmd {
    Config(Box<CameraCfg>),
    Armed(bool),
    Enroll(String),
    CancelEnroll,
    Forget(String),
    LearnOwner,
    Snapshot(String),
    MapRoom,
    ResetMap,
    DismissPortal(usize),
    RelearnDoor,
    SetHome,
    Rescan,
    Shutdown,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum CamStatus {
    #[default]
    Starting,
    Off,
    Running,
    Waiting(String),
    Failed(String),
    NoPermission,
}

#[derive(Clone, Debug, Default)]
pub struct TrackView {
    pub id: u32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
    pub known: bool,
    pub path: Vec<(f32, f32)>,
    pub predicted: Vec<(f32, f32)>,
    pub lost: bool,
    /// 17 COCO body joints (x, y, confidence) in view coordinates.
    pub keypoints: Vec<(f32, f32, f32)>,
}

#[derive(Clone, Debug, Default)]
pub struct FaceView {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub label: String,
    pub known: bool,
}

#[derive(Clone, Default)]
pub struct Overlay {
    pub tracks: Vec<TrackView>,
    pub faces: Vec<FaceView>,
    pub doorway: Option<NRect>,
    pub ignore: Vec<NRect>,
    pub motion: Vec<f32>,
    pub mgw: usize,
    pub mgh: usize,
    pub door_open: bool,
    pub covered: bool,
    pub light_change: bool,
    pub focus: Option<NRect>,
    pub ptz_line: String,
    pub zone_active: [bool; 2],
    /// People are confirmed by body and skeleton detection, not raw motion.
    pub body_mode: bool,
    /// pose::POSE_LOADING, POSE_READY or POSE_FAILED.
    pub pose_state: u8,
    /// How long the last body tracking round took.
    pub pose_ms: f32,
}

pub struct Preview {
    pub seq: u64,
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<u8>,
    pub ov: Overlay,
}

#[derive(Clone, Debug, Default)]
pub struct EnrollView {
    pub name: String,
    pub prompt: String,
    pub progress: f32,
    pub done: bool,
    pub error: Option<String>,
}

#[derive(Clone, Default)]
pub struct MapView {
    pub heat: Vec<f32>,
    pub floor: Vec<f32>,
    pub portals: Vec<map::Portal>,
    pub doorway_hint: Option<NRect>,
    pub ignore_hints: Vec<NRect>,
    pub tracks_learned: u32,
    pub version: u64,
}

#[derive(Default)]
pub struct CamShared {
    pub preview: Option<Arc<Preview>>,
    pub status: CamStatus,
    pub devices: Vec<(u32, String)>,
    pub active_name: String,
    pub fps: f32,
    pub enroll: Option<EnrollView>,
    pub people: Vec<(String, usize)>,
    pub map: MapView,
    pub pano: Option<Arc<frame::Rgb>>,
    pub pano_seq: u64,
    pub xf: Option<ptz::Xf>,
    pub ptz_status: String,
    pub mapping_progress: Option<f32>,
    pub home_pose: Option<(f32, f32)>,
    pub last_frame_ms: i64,
    pub door_learned: bool,
}

#[derive(Clone)]
pub struct CameraHandle {
    pub tx: Sender<CamCmd>,
    pub shared: Arc<Mutex<CamShared>>,
    done: Arc<Mutex<Option<Receiver<()>>>>,
}

impl CameraHandle {
    pub fn send(&self, c: CamCmd) {
        let _ = self.tx.send(c);
    }

    /// Waits (briefly) for the camera thread to save its room map and stop.
    pub fn wait_stopped(&self, max: Duration) {
        if let Ok(mut g) = self.done.lock() {
            if let Some(rx) = g.take() {
                let _ = rx.recv_timeout(max);
            }
        }
    }
}

pub fn start(cfg: CameraCfg, engine: Sender<EngineMsg>, ctx: egui::Context) -> CameraHandle {
    let (tx, rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let shared = Arc::new(Mutex::new(CamShared::default()));
    let s2 = shared.clone();
    let spawned = std::thread::Builder::new().name("camera".into()).spawn(move || {
        let worker = Worker::new(cfg, engine, ctx, s2);
        worker.run(rx);
        let _ = done_tx.send(());
    });
    if let Err(e) = spawned {
        util::log(&format!("could not start the camera thread: {}", e));
    }
    CameraHandle { tx, shared, done: Arc::new(Mutex::new(Some(done_rx))) }
}

struct Enroll {
    name: String,
    samples: Vec<Vec<u8>>,
    last_at: Option<Instant>,
    last_center: (f32, f32),
    started: Instant,
    candidate: Option<(Vec<u8>, (f32, f32), f32)>,
}

struct Worker {
    cfg: CameraCfg,
    engine: Sender<EngineMsg>,
    ctx: egui::Context,
    shared: Arc<Mutex<CamShared>>,
    cam: Option<nokhwa::Camera>,
    cam_key: String,
    retry_at: Instant,
    fails: u32,
    bad_decodes: u32,
    motion: motion::Motion,
    tracker: track::Tracker,
    map: map::RoomMap,
    map_dirty_at: Option<Instant>,
    map_version: u64,
    door: map::DoorWatch,
    cascade: Option<faces::Cascade>,
    db: faces::FaceDb,
    gallery: faces::Gallery,
    enroll: Option<Enroll>,
    ptz: ptz::Ptz,
    sweep: Option<ptz::Sweep>,
    armed: bool,
    zone_open: [bool; 2],
    zone_hits: [u32; 2],
    zone_last: [Option<Instant>; 2],
    sent_camera: Option<(bool, bool)>,
    covered: bool,
    cover_since: Option<Instant>,
    std_avg: f32,
    seq: u64,
    frame_no: u64,
    fps_t: Instant,
    fps_n: u32,
    recent_face: Option<(Instant, Vec<u8>)>,
    face_obs: VecDeque<(Instant, Option<usize>, f32, (f32, f32))>,
    last_recognized: Option<Instant>,
    last_stranger: Option<Instant>,
    focus: Option<NRect>,
    last_motion_at: Option<Instant>,
    permission_ok: bool,
    last_faces: Vec<FaceView>,
    last_faces_at: Option<Instant>,
    last_frame_at: Option<Instant>,
    last_repaint_at: Option<Instant>,
    snap_queue: Vec<String>,
    demo_t: f32,
    pose: Option<pose::PoseWorker>,
    people: Vec<pose::Person>,
    people_at: Option<Instant>,
    pose_sent_at: Option<Instant>,
    pose_ms: f32,
}

impl Worker {
    fn new(cfg: CameraCfg, engine: Sender<EngineMsg>, ctx: egui::Context, shared: Arc<Mutex<CamShared>>) -> Worker {
        let cascade = faces::Cascade::load(include_bytes!("../../assets/cascade/lbp_frontalface.bin"));
        if cascade.is_none() {
            util::log("the face cascade failed to load, face features are off");
        }
        let db = faces::FaceDb::load();
        let gallery = db.gallery();
        let ptz = ptz::Ptz::new(&cfg.ptz, cfg.lens.hfov());
        let now = Instant::now();
        let w = Worker {
            cfg,
            engine,
            ctx,
            shared,
            cam: None,
            cam_key: String::new(),
            retry_at: now,
            fails: 0,
            bad_decodes: 0,
            motion: motion::Motion::new(),
            tracker: track::Tracker::new(),
            map: map::RoomMap::default(),
            map_dirty_at: None,
            map_version: 1,
            door: map::DoorWatch::new(),
            cascade,
            db,
            gallery,
            enroll: None,
            ptz,
            sweep: None,
            armed: false,
            zone_open: [false; 2],
            zone_hits: [0; 2],
            zone_last: [None; 2],
            sent_camera: None,
            covered: false,
            cover_since: None,
            std_avg: 20.0,
            seq: 0,
            frame_no: 0,
            fps_t: now,
            fps_n: 0,
            recent_face: None,
            face_obs: VecDeque::new(),
            last_recognized: None,
            last_stranger: None,
            focus: None,
            last_motion_at: None,
            permission_ok: false,
            last_faces: Vec::new(),
            last_faces_at: None,
            last_frame_at: None,
            last_repaint_at: None,
            snap_queue: Vec::new(),
            demo_t: 0.0,
            pose: pose::PoseWorker::start(),
            people: Vec::new(),
            people_at: None,
            pose_sent_at: None,
            pose_ms: 0.0,
        };
        w.publish_people();
        if let Ok(mut s) = w.shared.lock() {
            s.devices.push((DEMO_DEVICE, "Demo camera (no webcam needed)".to_string()));
        }
        w
    }

    fn run(mut self, rx: Receiver<CamCmd>) {
        loop {
            loop {
                match rx.try_recv() {
                    Ok(cmd) => {
                        if !self.command(cmd) {
                            self.save_map();
                            return;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.save_map();
                        return;
                    }
                }
            }
            if !self.cfg.enabled {
                if self.cam.is_some() {
                    self.close();
                }
                self.set_status(CamStatus::Off);
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            if self.cfg.device == DEMO_DEVICE {
                if self.cam.is_some() {
                    self.close();
                }
                self.demo_step();
                continue;
            }
            if self.cam.is_none() {
                if Instant::now() < self.retry_at {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                self.open();
                if self.cam.is_none() {
                    continue;
                }
            }
            let result = match self.cam.as_mut() {
                Some(c) => c.frame(),
                None => continue,
            };
            match result {
                Ok(buf) => {
                    self.fails = 0;
                    // Cap camera processing to ~15 FPS so high-framerate webcams never
                    // saturate CPU/GPU or freeze the UI thread.
                    let now = Instant::now();
                    if let Some(prev) = self.last_frame_at {
                        let elapsed = now.saturating_duration_since(prev);
                        if elapsed < Duration::from_millis(62) {
                            std::thread::sleep(Duration::from_millis(62).saturating_sub(elapsed).min(Duration::from_millis(25)));
                            continue;
                        }
                    }
                    self.last_frame_at = Some(now);
                    let res = buf.resolution();
                    let hint = hint_of(buf.source_frame_format());
                    let decoded = frame::decode(buf.buffer(), res.width() as usize, res.height() as usize, hint, 480);
                    match decoded {
                        Some(mut rgb) => {
                            self.bad_decodes = 0;
                            if self.cfg.mirror {
                                rgb.mirror();
                            }
                            self.process(rgb);
                        }
                        None => {
                            self.bad_decodes += 1;
                            if self.bad_decodes == 30 {
                                util::log("the camera is sending frames RoomGuard can't decode");
                                self.set_status(CamStatus::Failed("The camera's picture format isn't supported".into()));
                            }
                        }
                    }
                }
                Err(e) => {
                    self.fails += 1;
                    if self.fails == 1 || self.fails % 40 == 0 {
                        util::log(&format!("camera frame failed: {}", e));
                    }
                    if self.fails > 25 {
                        self.close();
                        self.report_camera(false, false);
                        self.set_status(CamStatus::Waiting("The camera stopped sending pictures. Retrying".into()));
                        self.retry_at = Instant::now() + Duration::from_secs(3);
                    } else {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
            }
        }
    }

    fn command(&mut self, cmd: CamCmd) -> bool {
        match cmd {
            CamCmd::Config(c) => {
                let c = *c;
                let reopen = c.device != self.cfg.device || (c.enabled && !self.cfg.enabled);
                let ptz_changed = c.ptz != self.cfg.ptz || c.lens != self.cfg.lens;
                let door_changed = c.doorway != self.cfg.doorway;
                self.cfg = c;
                if reopen {
                    self.close();
                    self.retry_at = Instant::now();
                }
                if ptz_changed {
                    self.ptz = ptz::Ptz::new(&self.cfg.ptz, self.cfg.lens.hfov());
                    self.load_ptz();
                }
                if door_changed {
                    self.door.relearn();
                }
            }
            CamCmd::Armed(a) => self.armed = a,
            CamCmd::Enroll(name) => {
                let err = if !self.cfg.enabled {
                    Some("Turn on the camera above before learning a face.")
                } else if self.cfg.device == DEMO_DEVICE {
                    Some("Switch from the Demo camera to a real webcam to learn a face.")
                } else if self.cam.is_none() {
                    Some("The camera isn't ready yet. Wait for the live picture and try again.")
                } else if self.cascade.is_none() {
                    Some("Face detector didn't load on this build.")
                } else {
                    None
                };
                if let Some(msg) = err {
                    self.enroll = None;
                    if let Ok(mut s) = self.shared.lock() {
                        s.enroll = Some(EnrollView {
                            name,
                            prompt: String::new(),
                            progress: 0.0,
                            done: false,
                            error: Some(msg.to_string()),
                        });
                    }
                    self.ctx.request_repaint();
                } else {
                    if let Ok(mut s) = self.shared.lock() {
                        s.enroll = Some(EnrollView {
                            name: name.clone(),
                            prompt: "Look straight at the camera".to_string(),
                            progress: 0.0,
                            done: false,
                            error: None,
                        });
                    }
                    self.enroll = Some(Enroll {
                        name,
                        samples: Vec::new(),
                        last_at: None,
                        last_center: (0.5, 0.5),
                        started: Instant::now(),
                        candidate: None,
                    });
                    self.ctx.request_repaint();
                }
            }
            CamCmd::CancelEnroll => {
                self.enroll = None;
                if let Ok(mut s) = self.shared.lock() {
                    s.enroll = None;
                }
            }
            CamCmd::Forget(name) => {
                self.db.remove(&name);
                self.db.save();
                self.gallery = self.db.gallery();
                self.publish_people();
            }
            CamCmd::LearnOwner => {
                let fresh = self
                    .recent_face
                    .as_ref()
                    .filter(|(t, _)| t.elapsed().as_secs_f32() < 8.0)
                    .map(|(_, d)| d.clone());
                let owner = self.db.people.first().map(|p| p.name.clone());
                if let (Some(d), Some(name)) = (fresh, owner) {
                    self.db.add_sample(&name, &d);
                    self.db.save();
                    self.gallery = self.db.gallery();
                    self.publish_people();
                }
            }
            CamCmd::Snapshot(reason) => self.snap_queue.push(reason),
            CamCmd::MapRoom => {
                if self.ptz.active() {
                    self.sweep = Some(ptz::Sweep::new(&mut self.ptz));
                }
            }
            CamCmd::ResetMap => {
                self.map.clear();
                self.map_version += 1;
                self.map_dirty_at = Some(Instant::now());
            }
            CamCmd::DismissPortal(i) => {
                self.map.dismiss_portal(i);
                self.map_version += 1;
                self.map_dirty_at = Some(Instant::now());
            }
            CamCmd::RelearnDoor => self.door.relearn(),
            CamCmd::SetHome => {
                self.cfg.ptz.home_pan = self.ptz.pan;
                self.cfg.ptz.home_tilt = self.ptz.tilt;
                self.ptz.cfg.home_pan = self.ptz.pan;
                self.ptz.cfg.home_tilt = self.ptz.tilt;
                if let Ok(mut s) = self.shared.lock() {
                    s.home_pose = Some((self.ptz.pan, self.ptz.tilt));
                }
            }
            CamCmd::Rescan => self.scan_devices(),
            CamCmd::Shutdown => return false,
        }
        true
    }

    fn set_status(&self, st: CamStatus) {
        if let Ok(mut s) = self.shared.lock() {
            if s.status != st {
                s.status = st;
                drop(s);
                self.ctx.request_repaint();
            }
        }
    }

    fn send_event(&self, ev: VisionEvent) {
        let _ = self.engine.send(EngineMsg::Vision(ev));
    }

    fn report_camera(&mut self, online: bool, covered: bool) {
        if self.sent_camera != Some((online, covered)) {
            self.sent_camera = Some((online, covered));
            self.send_event(VisionEvent::Camera { online, covered });
        }
    }

    fn scan_devices(&mut self) {
        match nokhwa::query(nokhwa::utils::ApiBackend::Auto) {
            Ok(list) => {
                let devices: Vec<(u32, String)> = list
                    .iter()
                    .enumerate()
                    .map(|(i, info)| {
                        let idx = match info.index() {
                            CameraIndex::Index(n) => *n,
                            _ => i as u32,
                        };
                        (idx, info.human_name())
                    })
                    .collect();
                if let Ok(mut s) = self.shared.lock() {
                    s.devices = devices;
                }
            }
            Err(e) => util::log(&format!("listing cameras failed: {}", e)),
        }
        if let Ok(mut s) = self.shared.lock() {
            if !s.devices.iter().any(|d| d.0 == DEMO_DEVICE) {
                s.devices.push((DEMO_DEVICE, "Demo camera (no webcam needed)".to_string()));
            }
        }
    }

    /// One frame from the demo camera: a drawn room with someone walking in
    /// and out, fed through exactly the same pipeline as a real camera.
    fn demo_step(&mut self) {
        std::thread::sleep(Duration::from_millis(66));
        if self.cam_key != "demo-camera" {
            self.save_map();
            self.cam_key = "demo-camera".to_string();
            self.load_map();
            self.load_ptz();
            self.motion.reset();
            self.tracker.clear();
            if let Ok(mut s) = self.shared.lock() {
                s.active_name = "Demo camera".to_string();
            }
            self.set_status(CamStatus::Running);
            self.report_camera(true, false);
        }
        self.demo_t += 1.0 / 15.0;
        let rgb = demo::scene(self.demo_t);
        self.process(rgb);
    }

    fn open(&mut self) {
        #[cfg(target_os = "macos")]
        {
            if !self.permission_ok {
                self.set_status(CamStatus::Waiting("Waiting for camera permission".into()));
                self.permission_ok = request_permission();
                if !self.permission_ok {
                    self.set_status(CamStatus::NoPermission);
                    self.retry_at = Instant::now() + Duration::from_secs(5);
                    return;
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.permission_ok = true;
        }
        self.scan_devices();
        self.set_status(CamStatus::Starting);
        match open_camera(self.cfg.device) {
            Ok(cam) => {
                self.cam = Some(cam);
                self.fails = 0;
                self.motion.reset();
                self.tracker.clear();
                let name = self
                    .shared
                    .lock()
                    .ok()
                    .and_then(|s| s.devices.iter().find(|d| d.0 == self.cfg.device).map(|d| d.1.clone()))
                    .unwrap_or_else(|| format!("Camera {}", self.cfg.device));
                util::log(&format!("camera opened: {}", name));
                if let Ok(mut s) = self.shared.lock() {
                    s.active_name = name.clone();
                }
                let key = util::slug(&name);
                if key != self.cam_key {
                    self.save_map();
                    self.cam_key = key;
                    self.load_map();
                    self.load_ptz();
                }
                self.set_status(CamStatus::Running);
                self.report_camera(true, false);
            }
            Err(e) => {
                util::log(&format!("camera {} could not open: {}", self.cfg.device, e));
                self.set_status(CamStatus::Failed(e));
                self.report_camera(false, false);
                self.retry_at = Instant::now() + Duration::from_secs(4);
            }
        }
    }

    fn close(&mut self) {
        if let Some(mut c) = self.cam.take() {
            let _ = c.stop_stream();
        }
    }

    fn maps_dir() -> PathBuf {
        util::data_dir().join("maps")
    }

    fn load_map(&mut self) {
        let path = Self::maps_dir().join(format!("{}.json", self.cam_key));
        self.map = util::load_json::<map::RoomMap>(&path).unwrap_or_default();
        self.map.ensure();
        self.map_version += 1;
        let pano_path = Self::maps_dir().join(format!("{}-pano.jpg", self.cam_key));
        let pano = std::fs::read(&pano_path).ok().and_then(|b| frame::decode_jpeg_file(&b));
        if let Ok(mut s) = self.shared.lock() {
            s.pano = pano.map(Arc::new);
            s.pano_seq += 1;
        }
    }

    fn save_map(&mut self) {
        if self.cam_key.is_empty() {
            return;
        }
        let path = Self::maps_dir().join(format!("{}.json", self.cam_key));
        util::save_json(&path, &self.map);
        self.map_dirty_at = None;
    }

    fn load_ptz(&mut self) {
        if self.cam_key.is_empty() {
            return;
        }
        let path = Self::maps_dir().join(format!("{}-ptz.json", self.cam_key));
        if let Some(s) = util::load_json::<ptz::PtzSave>(&path) {
            self.ptz.load_state(&s);
        }
    }

    fn process(&mut self, rgb: frame::Rgb) {
        let now = Instant::now();
        self.frame_no += 1;
        self.fps_n += 1;
        let since = now.saturating_duration_since(self.fps_t).as_secs_f32();
        if since >= 1.0 {
            let fps = self.fps_n as f32 / since;
            self.fps_n = 0;
            self.fps_t = now;
            if let Ok(mut s) = self.shared.lock() {
                s.fps = fps;
            }
        }
        let (w, h) = (rgb.w, rgb.h);
        let gray = rgb.gray();
        let mh = ((motion::MW * h) / w.max(1)).max(8);
        let small = frame::downscale_box(&gray, w, h, motion::MW, mh);
        let xf = self.ptz.xf();
        if self.ptz.is_moving(now) || self.sweep.is_some() {
            self.motion.settle(2);
        }
        let door_view = self.cfg.doorway.map(|r| xf.rect_map_to_view(r));
        let ignore_view: Vec<NRect> = self.cfg.ignore.iter().map(|r| xf.rect_map_to_view(*r)).collect();
        let m = self.motion.process(&small, motion::MW, mh, self.cfg.sensitivity, door_view, &ignore_view);
        if m.valid && m.total_frac > 0.002 {
            self.last_motion_at = Some(now);
        }
        self.check_cover(&m, now);

        let enrolling = self.enroll.is_some();
        let recent_motion = self
            .last_motion_at
            .map_or(false, |t| now.saturating_duration_since(t).as_secs_f32() < 3.0);
        let following = self.ptz.mode == ptz::Mode::Following || self.ptz.mode == ptz::Mode::Searching;

        // Humanoid body + skeleton detection. Only a real human body counts as
        // a person; raw pixel motion only decides how often and where to look.
        let demo = self.cfg.device == DEMO_DEVICE;
        let mut fresh: Option<Vec<pose::Person>> = None;
        if demo {
            fresh = Some(demo::people(self.demo_t));
        } else {
            let active = recent_motion || !self.tracker.tracks.is_empty() || enrolling || following;
            // About ten looks a second while anything is going on, about one
            // a second in a still, empty room. That keeps one CPU core from
            // being pinned while still catching anyone the moment they move.
            let gap = if active { 0.1 } else { 0.9 };
            let due = self.pose_sent_at.map_or(true, |t| now.saturating_duration_since(t).as_secs_f32() >= gap);
            let zooms = if due && self.pose.is_some() { self.zoom_targets(&m, w, h) } else { Vec::new() };
            let mut dead = false;
            if let Some(pw) = self.pose.as_ref() {
                loop {
                    match pw.rx.try_recv() {
                        Ok(r) => {
                            self.pose_ms = r.ms;
                            fresh = Some(r.people);
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            dead = true;
                            break;
                        }
                    }
                }
                if !dead && due && pw.submit(pose::PoseJob { img: Arc::new(frame::Rgb { w, h, data: rgb.data.clone() }), zooms }) {
                    self.pose_sent_at = Some(now);
                }
            }
            if dead {
                util::log("body tracking is unavailable, falling back to motion tracking");
                self.pose = None;
            }
        }
        let body_mode = demo || self.pose.is_some();
        self.tracker.confirm_hits = if body_mode { 3 } else { 5 };
        self.tracker.body_mode = body_mode;
        let det_thr = [0.55f32, 0.50, 0.45, 0.42, 0.38][(self.cfg.sensitivity.clamp(1, 5) - 1) as usize];
        let got_fresh = fresh.is_some();
        if let Some(list) = fresh {
            let ignore = &ignore_view;
            self.people = list
                .into_iter()
                .filter(|p| p.evidence(det_thr) >= pose::Evidence::Weak)
                // Someone on a TV or in a mirror inside an ignored area isn't in the room.
                .filter(|p| !ignore.iter().any(|r| r.contains(p.cx(), p.cy())))
                .collect();
            self.people_at = Some(now);
        } else if self.people_at.map_or(false, |t| now.saturating_duration_since(t).as_secs_f32() > 1.5) {
            self.people.clear();
        }

        let mut blobs: Vec<track::Blob> = if body_mode {
            if got_fresh {
                let motion_ok = m.valid && !m.light_change;
                self.people
                    .iter()
                    .map(|p| {
                        let strength = match p.evidence(det_thr) {
                            pose::Evidence::Strong => 3,
                            pose::Evidence::Normal => 2,
                            _ => 1,
                        };
                        let motion = if motion_ok { mask_frac(&self.motion.mask, self.motion.w, self.motion.h, p) } else { 0.0 };
                        track::Blob { cx: p.cx(), cy: p.cy(), w: p.w(), h: p.h(), px: 100, strength, motion }
                    })
                    .collect()
            } else {
                Vec::new()
            }
        } else if m.valid {
            let min_px = ((motion::MW * mh) / 140).max(20) as u32;
            track::find_blobs(&self.motion.mask, motion::MW, mh, min_px)
        } else {
            Vec::new()
        };

        let want_faces = self.cfg.faces
            && self.cascade.is_some()
            && (enrolling || if body_mode { !self.people.is_empty() } else { recent_motion || following });
        let every = if enrolling { 2 } else { 4 };
        if want_faces && self.frame_no % every == 0 {
            let regions: Option<Vec<NRect>> = if body_mode && !enrolling {
                Some(self.people.iter().map(|p| p.head_region()).collect())
            } else {
                None
            };
            self.last_faces = self.detect_faces(&gray, w, h, now, regions.as_deref());
            self.last_faces_at = Some(now);
            if !m.valid && !body_mode {
                // The picture is moving, so motion can't be trusted. Faces keep the track alive.
                for f in &self.last_faces {
                    blobs.push(track::Blob {
                        cx: f.x + f.w * 0.5,
                        cy: (f.y + f.h * 3.0).min(0.95),
                        w: f.w * 2.6,
                        h: f.h * 6.0,
                        px: 100,
                        strength: 0,
                        motion: 1.0,
                    });
                }
            }
        } else if self
            .last_faces_at
            .map_or(false, |t| now.saturating_duration_since(t).as_secs_f32() > 0.7)
        {
            self.last_faces.clear();
        }

        let (sx, sy) = xf.view_scale();
        if !body_mode || got_fresh {
            let blobs_map: Vec<track::Blob> = blobs
                .iter()
                .map(|b| {
                    let (u, v) = xf.view_to_map(b.cx, b.cy);
                    track::Blob { cx: u, cy: v, w: b.w * sx, h: b.h * sy, ..*b }
                })
                .collect();
            self.tracker.update(&blobs_map, now);
        }
        for t in self.tracker.tracks.iter_mut() {
            let (tx, ty) = t.pos();
            for f in &self.last_faces {
                let (fx, fy) = xf.view_to_map(f.x + f.w * 0.5, f.y + f.h * 0.5);
                if (fx - tx).abs() < t.w * 0.75 + 0.03 && (fy - ty).abs() < t.h * 0.75 + 0.05 && !f.label.is_empty() {
                    t.label = Some(f.label.clone());
                    t.known = f.known;
                }
            }
        }

        let finished = std::mem::take(&mut self.tracker.finished);
        if self.cfg.learn_map && self.sweep.is_none() {
            for f in &finished {
                if !self.armed || !f.known {
                    self.map.learn(f);
                }
            }
            if !finished.is_empty() {
                self.map_version += 1;
                if self.map_dirty_at.is_none() {
                    self.map_dirty_at = Some(now);
                }
            }
            if m.valid && !xf.active {
                let grid = self.motion.grid(map::GW, map::GH);
                let people: Vec<(f32, f32)> = self
                    .tracker
                    .tracks
                    .iter()
                    .filter(|t| t.confirmed)
                    .map(|t| t.foot())
                    .collect();
                self.map.observe(&grid, &people);
            }
        }

        if let (Some(dv), Some(dm)) = (door_view, self.cfg.doorway) {
            let inside = dv.x >= -0.02 && dv.y >= -0.02 && dv.x + dv.w <= 1.02 && dv.y + dv.h <= 1.02;
            if self.cfg.door_contact && inside && !self.ptz.is_moving(now) {
                let someone = self.tracker.tracks.iter().any(|t| {
                    let (fx, fy) = t.foot();
                    t.confirmed && t.unseen(now) < 0.6 && (dm.contains(fx, fy) || dm.contains(t.pos().0, t.pos().1))
                });
                let moving = !m.valid || m.door_frac > 0.003 || someone;
                if let Some(open) = self.door.update(&gray, w, h, dv, moving, now) {
                    self.send_event(VisionEvent::DoorOpen(open));
                }
            }
        }

        self.update_zones(&m, now, body_mode);
        self.recognition(now);
        self.enroll_step(now);
        self.drive_ptz(&rgb, &small, mh, now);
        self.auto_frame(&xf, now);
        if !self.snap_queue.is_empty() {
            let reasons = std::mem::take(&mut self.snap_queue);
            for r in reasons {
                self.save_snapshot(&rgb, &r);
            }
        }
        if self.map_dirty_at.map_or(false, |t| now.saturating_duration_since(t).as_secs_f32() > 60.0) {
            self.save_map();
        }
        self.publish(rgb, &m, &xf, now);
    }

    /// Where a closer look could find someone the whole frame can't: moving
    /// areas that no body explains yet (someone far away arriving), then small
    /// people already being followed, so they stay followed.
    fn zoom_targets(&self, m: &motion::MotionOut, fw: usize, fh: usize) -> Vec<NRect> {
        let mut out: Vec<NRect> = Vec::new();
        let explained = |cx: f32, cy: f32| {
            self.people
                .iter()
                .any(|p| cx >= p.x0 - 0.02 && cx <= p.x1 + 0.02 && cy >= p.y0 - 0.02 && cy <= p.y1 + 0.02)
        };
        if m.valid && !m.light_change {
            let (mw, mh) = (self.motion.w, self.motion.h);
            let min_px = ((mw * mh) / 900).max(4) as u32;
            let mut moving = track::components(&self.motion.mask, mw, mh, min_px);
            moving.sort_by(|a, b| b.px.cmp(&a.px));
            for b in &moving {
                if out.len() >= pose::MAX_ZOOMS {
                    break;
                }
                if explained(b.cx, b.cy) || out.iter().any(|o| o.contains(b.cx, b.cy)) {
                    continue;
                }
                if let Some(r) = zoom_rect(b.cx, b.cy, b.w, b.h, fw, fh) {
                    out.push(r);
                }
            }
        }
        for p in &self.people {
            if out.len() >= pose::MAX_ZOOMS {
                break;
            }
            if p.h() < 0.5 && !out.iter().any(|o| o.contains(p.cx(), p.cy())) {
                if let Some(r) = zoom_rect(p.cx(), p.cy(), p.w(), p.h(), fw, fh) {
                    out.push(r);
                }
            }
        }
        out
    }

    fn check_cover(&mut self, m: &motion::MotionOut, now: Instant) {
        let flat = m.std < 3.0;
        if flat {
            if self.cover_since.is_none() && self.std_avg > 12.0 {
                self.cover_since = Some(now);
            }
        } else {
            self.cover_since = None;
            self.std_avg = self.std_avg * 0.9 + m.std * 0.1;
            if self.covered {
                self.covered = false;
                self.report_camera(true, false);
            }
        }
        if let Some(t) = self.cover_since {
            if !self.covered && now.saturating_duration_since(t).as_secs_f32() > 3.0 {
                self.covered = true;
                self.report_camera(true, true);
                self.snap_queue.push("camera-covered".to_string());
            }
        }
        if !self.covered && self.sent_camera != Some((true, false)) {
            self.report_camera(true, false);
        }
    }

    fn update_zones(&mut self, m: &motion::MotionOut, now: Instant, body_mode: bool) {
        let thr = [0.012f32, 0.008, 0.005, 0.0035, 0.002][(self.cfg.sensitivity.clamp(1, 5) - 1) as usize];
        let door_rect = self.cfg.doorway;
        let mut door_active = false;
        let mut room_active = false;
        // With body tracking, raw pixel motion never trips a zone on its own:
        // only a confirmed human body (or the door watcher) does.
        if m.valid && !body_mode {
            if door_rect.is_some() {
                door_active |= m.door_frac > thr * 1.5;
                room_active |= m.room_frac > thr;
            } else {
                door_active |= m.total_frac > thr;
            }
        }
        let max_unseen = if body_mode { 0.6 } else { 0.3 };
        for t in &self.tracker.tracks {
            if !t.confirmed || t.unseen(now) > max_unseen {
                continue;
            }
            let (fx, fy) = t.foot();
            // The room works like a real interior motion detector: it sees
            // people moving, not people sitting still. That's what lets you
            // stand at the keypad and get a Ready light, just like with a PIR.
            // Someone in the doorway holds the door zone open like a door would.
            let moving = !body_mode || t.moved_within(now, 1.2);
            match door_rect {
                Some(d) if d.contains(fx, fy) || d.contains(t.pos().0, t.pos().1) => door_active = true,
                Some(_) => room_active |= moving,
                None => door_active |= moving,
            }
        }
        if self.door.open && self.cfg.door_contact {
            door_active = true;
        }
        let actives = [door_active, room_active];
        for i in 0..2 {
            if actives[i] {
                self.zone_hits[i] += 1;
                self.zone_last[i] = Some(now);
            } else {
                self.zone_hits[i] = 0;
            }
            let hold = if i == 0 && self.door.open { 0.2 } else { 2.5 };
            let want_open = if self.zone_open[i] {
                self.zone_last[i].map_or(false, |t| now.saturating_duration_since(t).as_secs_f32() < hold)
            } else {
                self.zone_hits[i] >= 3
            };
            if want_open != self.zone_open[i] {
                self.zone_open[i] = want_open;
                self.send_event(VisionEvent::Motion { zone: i as u8 + 1, open: want_open });
            }
        }
    }

    fn detect_faces(&mut self, gray: &[u8], w: usize, h: usize, now: Instant, regions: Option<&[NRect]>) -> Vec<FaceView> {
        let mut rects = match self.cascade.as_ref() {
            Some(c) => {
                // 320px pyramid with 1.22 scale step runs ~3.5x faster than 480px/1.15
                // while keeping full frontal face detection reliability.
                let dw = w.min(320).max(1);
                let scale = w as f32 / dw as f32;
                let dh = ((h as f32) / scale).max(1.0) as usize;
                let det = if dw == w { gray.to_vec() } else { frame::resize_bilinear(gray, w, h, dw, dh) };
                c.detect(&det, dw, dh, 24.0, 1.22, 3)
                    .into_iter()
                    .map(|r| faces::FaceRect { x: r.x * scale, y: r.y * scale, w: r.w * scale, h: r.h * scale, votes: r.votes })
                    .collect::<Vec<_>>()
            }
            None => return Vec::new(),
        };
        // Sort largest face first so enrollment always picks the primary person in front of the camera.
        rects.sort_by(|a, b| (b.w * b.h).partial_cmp(&(a.w * a.h)).unwrap_or(std::cmp::Ordering::Equal));
        let mut out = Vec::new();
        for (idx, full) in rects.iter().take(4).enumerate() {
            let center = ((full.x + full.w * 0.5) / w as f32, (full.y + full.h * 0.5) / h as f32);
            // A face only counts where a real body's head can be.
            if let Some(rs) = regions {
                if !rs.iter().any(|r| r.contains(center.0, center.1)) {
                    continue;
                }
            }
            let mut label = String::new();
            let mut known = false;
            if let Some(desc) = faces::describe(gray, w, h, full) {
                match self.gallery.best(&desc, self.cfg.strictness) {
                    Some((pi, ratio)) => {
                        self.face_obs.push_back((now, Some(pi), ratio, center));
                        if ratio <= 1.0 {
                            label = self.gallery.people[pi].0.clone();
                            known = true;
                        } else if ratio > 1.15 {
                            label = "Unknown".to_string();
                        }
                    }
                    None => {
                        self.face_obs.push_back((now, None, 9.0, center));
                        label = "Unknown".to_string();
                    }
                }
                if idx == 0 {
                    if let Some(e) = self.enroll.as_mut() {
                        e.candidate = Some((desc.clone(), center, full.w / w as f32));
                    }
                }
                self.recent_face = Some((now, desc));
            }
            out.push(FaceView {
                x: full.x / w as f32,
                y: full.y / h as f32,
                w: full.w / w as f32,
                h: full.h / h as f32,
                label,
                known,
            });
        }
        while self
            .face_obs
            .front()
            .map_or(false, |o| now.saturating_duration_since(o.0).as_secs_f32() > 3.0)
        {
            self.face_obs.pop_front();
        }
        out
    }

    fn face_is_live(&self, now: Instant) -> bool {
        let pts: Vec<(f32, f32)> = self
            .face_obs
            .iter()
            .filter(|o| now.saturating_duration_since(o.0).as_secs_f32() < 2.5)
            .map(|o| o.3)
            .collect();
        if pts.len() < 2 {
            return false;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (1.0f32, 1.0f32, 0.0f32, 0.0f32);
        for p in &pts {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
        (x1 - x0).max(y1 - y0) > 0.009
    }

    fn recognition(&mut self, now: Instant) {
        let recent: Vec<(Option<usize>, f32)> = self.face_obs.iter().rev().take(5).map(|o| (o.1, o.2)).collect();
        if recent.len() < 3 {
            return;
        }
        let matches = recent.iter().filter(|o| o.1 <= 1.0).count();
        let strangers = recent.iter().filter(|o| o.1 > 1.15).count();
        let live = !self.cfg.liveness || self.face_is_live(now);
        if matches >= 3 && live {
            let due = self.last_recognized.map_or(true, |t| now.saturating_duration_since(t).as_secs_f32() > 5.0);
            let person = recent.iter().find(|o| o.1 <= 1.0).and_then(|o| o.0);
            if let (true, Some(pi)) = (due, person) {
                let name = self.gallery.people.get(pi).map(|p| p.0.clone()).unwrap_or_default();
                self.last_recognized = Some(now);
                self.send_event(VisionEvent::Recognized { name });
            }
        } else if strangers >= 4 && recent.len() >= 5 {
            let due = self.last_stranger.map_or(true, |t| now.saturating_duration_since(t).as_secs_f32() > 10.0);
            if due {
                self.last_stranger = Some(now);
                self.send_event(VisionEvent::Stranger);
            }
        }
    }

    fn enroll_step(&mut self, now: Instant) {
        let (done, view) = {
            let Some(en) = self.enroll.as_mut() else {
                return;
            };
            let mut too_far = false;
            if let Some((desc, center, size)) = en.candidate.take() {
                if size <= 0.055 {
                    too_far = true;
                } else {
                    let moved = (center.0 - en.last_center.0).abs() + (center.1 - en.last_center.1).abs();
                    let gap = en.last_at.map_or(f32::MAX, |t| now.saturating_duration_since(t).as_secs_f32());
                    if gap > 0.22 && (en.samples.is_empty() || moved > 0.007 || gap > 0.5) {
                        en.samples.push(desc);
                        en.last_at = Some(now);
                        en.last_center = center;
                    }
                }
            }
            let n = en.samples.len();
            let idle_s = en
                .last_at
                .unwrap_or(en.started)
                .elapsed()
                .as_secs_f32();
            let prompt = if too_far {
                "Move a little closer to the camera".to_string()
            } else if n == 0 && idle_s > 3.5 {
                "Face the camera in good light (waiting for a clear face)".to_string()
            } else if n < 6 {
                format!("Look straight at the camera ({}/{})", n, ENROLL_TARGET)
            } else if n < 12 {
                format!("Turn your head a little to your left ({}/{})", n, ENROLL_TARGET)
            } else if n < 18 {
                format!("Now a little to your right ({}/{})", n, ENROLL_TARGET)
            } else {
                format!("Tilt your chin slightly, then relax ({}/{})", n, ENROLL_TARGET)
            };
            let stalled = idle_s > 25.0;
            let error = if stalled {
                Some("I couldn't see a clear face for 25 seconds. Move closer, face the light, and try again.".to_string())
            } else {
                None
            };
            let view = EnrollView {
                name: en.name.clone(),
                prompt,
                progress: n as f32 / ENROLL_TARGET as f32,
                done: n >= ENROLL_TARGET,
                error,
            };
            (n >= ENROLL_TARGET || stalled, view)
        };
        if done {
            if let Some(en) = self.enroll.take() {
                if en.samples.len() >= ENROLL_TARGET {
                    self.db.set_person(&en.name, &en.samples);
                    self.db.save();
                    self.gallery = self.db.gallery();
                    self.publish_people();
                    util::log(&format!("learned {}'s face from {} samples", en.name, en.samples.len()));
                }
            }
        }
        if let Ok(mut s) = self.shared.lock() {
            s.enroll = Some(view);
        }
    }

    fn publish_people(&self) {
        let people: Vec<(String, usize)> = self.db.people.iter().map(|p| (p.name.clone(), p.samples.len())).collect();
        if let Ok(mut s) = self.shared.lock() {
            s.people = people;
        }
    }

    fn drive_ptz(&mut self, rgb: &frame::Rgb, small: &[u8], mh: usize, now: Instant) {
        if !self.ptz.active() {
            return;
        }
        if let Some(mut sw) = self.sweep.take() {
            let tile = rgb.shrink(4);
            let finished = sw.tick(&mut self.ptz, self.cam.as_mut(), small, motion::MW, mh, &tile, now);
            if finished {
                let pano = sw.finish(&mut self.ptz);
                let dir = Self::maps_dir();
                util::save_json(&dir.join(format!("{}-ptz.json", self.cam_key)), &self.ptz.save_state());
                if let Some(p) = &pano {
                    if let Some(bytes) = frame::encode_jpeg(p, 85) {
                        let _ = std::fs::create_dir_all(&dir);
                        let _ = std::fs::write(dir.join(format!("{}-pano.jpg", self.cam_key)), bytes);
                    }
                }
                self.map.clear();
                self.map_version += 1;
                self.tracker.clear();
                self.motion.reset();
                let home = (self.cfg.ptz.home_pan, self.cfg.ptz.home_tilt);
                self.ptz.send(self.cam.as_mut(), home.0, home.1, 50, now);
                if let Ok(mut s) = self.shared.lock() {
                    s.pano = pano.map(Arc::new);
                    s.pano_seq += 1;
                    s.mapping_progress = None;
                }
            } else {
                let progress = sw.progress();
                self.sweep = Some(sw);
                if let Ok(mut s) = self.shared.lock() {
                    s.mapping_progress = Some(progress);
                }
            }
            return;
        }
        let xf = self.ptz.xf();
        let span_az = xf.az_max - xf.az_min;
        let span_el = xf.el_max - xf.el_min;
        let mut target = None;
        let mut candidates: Vec<(f32, f32, &'static str)> = Vec::new();
        if let Some(t) = self.tracker.primary(now) {
            let (u, v) = t.pos();
            let (vu, vv) = t.vel();
            if t.unseen(now) < 0.4 {
                let (az, el) = xf.map_to_scene(u, v);
                target = Some(ptz::Target { az, el, vaz: vu * span_az, vel: -vv * span_el });
            }
            for (cx, cy, _) in self.map.likely_exits(u, v, vu, vv).into_iter().take(2) {
                let (a, e) = xf.map_to_scene(cx, cy);
                candidates.push((a, e, "Checking the doorway they were heading for"));
            }
            if let Some((sx, sy)) = self.map.next_steps(u, v, vu, vv, 6).last().copied() {
                let (a, e) = xf.map_to_scene(sx, sy);
                candidates.push((a, e, "Checking where people usually go from there"));
            }
        }
        self.ptz.follow(self.cam.as_mut(), target, &candidates, now);
    }

    fn auto_frame(&mut self, xf: &ptz::Xf, now: Instant) {
        if !self.cfg.auto_frame || xf.active {
            self.focus = None;
            return;
        }
        let want = self
            .tracker
            .primary(now)
            .filter(|t| t.unseen(now) < 1.0)
            .map(|t| {
                let (x, y) = t.pos();
                let size = (t.h * 1.7).clamp(0.4, 1.0);
                NRect::new(
                    (x - size * 0.5).clamp(0.0, 1.0 - size),
                    (y - size * 0.5).clamp(0.0, 1.0 - size),
                    size,
                    size,
                )
            })
            .unwrap_or(NRect::new(0.0, 0.0, 1.0, 1.0));
        let cur = self.focus.unwrap_or(NRect::new(0.0, 0.0, 1.0, 1.0));
        let k = 0.12;
        let next = NRect::new(
            cur.x + (want.x - cur.x) * k,
            cur.y + (want.y - cur.y) * k,
            cur.w + (want.w - cur.w) * k,
            cur.h + (want.h - cur.h) * k,
        );
        self.focus = if next.w > 0.995 { None } else { Some(next) };
    }

    fn save_snapshot(&self, rgb: &frame::Rgb, reason: &str) {
        let dir = util::data_dir().join("snapshots");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(format!("{}_{}.jpg", util::file_stamp(util::now_ms()), util::slug(reason)));
        match frame::encode_jpeg(rgb, 88) {
            Some(bytes) => match std::fs::write(&path, bytes) {
                Ok(()) => {
                    self.send_event(VisionEvent::Snapshot { path, reason: reason.to_string() });
                    prune_snapshots(&dir, 300);
                }
                Err(e) => util::log(&format!("could not save a snapshot: {}", e)),
            },
            None => util::log("could not encode a snapshot"),
        }
    }

    fn publish(&mut self, rgb: frame::Rgb, m: &motion::MotionOut, xf: &ptz::Xf, now: Instant) {
        self.seq += 1;
        let (sx, sy) = xf.view_scale();
        let mut ov = Overlay {
            motion: self.motion.grid(32, 24),
            mgw: 32,
            mgh: 24,
            faces: self.last_faces.clone(),
            doorway: self.cfg.doorway.map(|r| xf.rect_map_to_view(r)),
            ignore: self.cfg.ignore.iter().map(|r| xf.rect_map_to_view(*r)).collect(),
            door_open: self.door.open,
            covered: self.covered,
            light_change: m.light_change,
            focus: self.focus,
            zone_active: self.zone_open,
            body_mode: self.cfg.device == DEMO_DEVICE || self.pose.is_some(),
            pose_state: if self.cfg.device == DEMO_DEVICE {
                pose::POSE_READY
            } else {
                self.pose.as_ref().map_or(pose::POSE_FAILED, |p| p.state())
            },
            pose_ms: self.pose_ms,
            ..Default::default()
        };
        let people_fresh = self.people_at.map_or(false, |t| now.saturating_duration_since(t).as_secs_f32() < 0.8);
        for t in self.tracker.tracks.iter().filter(|t| t.confirmed) {
            let (x, y) = xf.map_to_view(t.pos().0, t.pos().1);
            let (tw, th) = (t.w / sx, t.h / sy);
            let keypoints = if people_fresh {
                self.people
                    .iter()
                    .map(|p| (p, (p.cx() - x).abs() + (p.cy() - y).abs()))
                    .filter(|(_, d)| *d < tw.max(th).max(0.08))
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(p, _)| p.kp.to_vec())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            ov.tracks.push(TrackView {
                id: t.id,
                x,
                y,
                w: tw,
                h: th,
                label: t.label.clone().unwrap_or_default(),
                known: t.known,
                path: t.path.iter().rev().take(40).map(|p| xf.map_to_view(p.0, p.1)).collect(),
                predicted: t.predicted_path(5, 1.2).into_iter().map(|p| xf.map_to_view(p.0, p.1)).collect(),
                lost: t.unseen(now) > 0.25,
                keypoints,
            });
        }
        if self.ptz.active() {
            ov.ptz_line = format!("{}: {}", self.ptz.mode.label(), self.ptz.why);
        }
        let frame::Rgb { w, h, data } = rgb;
        let preview = Arc::new(Preview { seq: self.seq, w, h, rgb: data, ov });
        let map_view = if self.frame_no % 30 == 0 {
            Some(MapView {
                heat: self.map.heat.clone(),
                floor: self.map.floor.clone(),
                portals: self.map.portals.clone(),
                doorway_hint: self.map.suggest_doorway(),
                ignore_hints: self.map.suggest_ignore(),
                tracks_learned: self.map.tracks,
                version: self.map_version,
            })
        } else {
            None
        };
        if let Ok(mut s) = self.shared.lock() {
            s.preview = Some(preview);
            s.last_frame_ms = util::now_ms();
            s.xf = Some(xf.clone());
            s.ptz_status = self.ptz.status.clone();
            s.door_learned = self.door.learned;
            if let Some(mv) = map_view {
                if mv.version != s.map.version || self.cfg.learn_map {
                    s.map = mv;
                }
            }
        }
        if self.last_repaint_at.map_or(true, |t| now.saturating_duration_since(t) >= Duration::from_millis(66)) {
            self.last_repaint_at = Some(now);
            self.ctx.request_repaint();
        }
    }
}

/// A square close look around a target, in normalized frame coordinates,
/// or None when the target is already big enough for the whole frame look.
fn zoom_rect(cx: f32, cy: f32, tw: f32, th: f32, fw: usize, fh: usize) -> Option<NRect> {
    let (fw, fh) = (fw.max(1) as f32, fh.max(1) as f32);
    let short = fw.min(fh);
    let side = ((th * fh).max(tw * fw) * 2.2).clamp(short * 0.25, short);
    if side >= short * 0.95 {
        return None;
    }
    let (sw, sh) = (side / fw, side / fh);
    let x = (cx - sw * 0.5).clamp(0.0, (1.0 - sw).max(0.0));
    let y = (cy - sh * 0.5).clamp(0.0, (1.0 - sh).max(0.0));
    Some(NRect::new(x, y, sw, sh))
}

/// Share of a person's box where the motion mask saw change this frame.
fn mask_frac(mask: &[u8], mw: usize, mh: usize, p: &pose::Person) -> f32 {
    if mw == 0 || mh == 0 || mask.len() < mw * mh {
        return 0.0;
    }
    let gx0 = ((p.x0.clamp(0.0, 1.0) * mw as f32) as usize).min(mw - 1);
    let gy0 = ((p.y0.clamp(0.0, 1.0) * mh as f32) as usize).min(mh - 1);
    let gx1 = ((p.x1.clamp(0.0, 1.0) * mw as f32).ceil() as usize).clamp(gx0 + 1, mw);
    let gy1 = ((p.y1.clamp(0.0, 1.0) * mh as f32).ceil() as usize).clamp(gy0 + 1, mh);
    let mut n = 0u32;
    for y in gy0..gy1 {
        for x in gx0..gx1 {
            if mask[y * mw + x] != 0 {
                n += 1;
            }
        }
    }
    n as f32 / ((gx1 - gx0) * (gy1 - gy0)).max(1) as f32
}

fn prune_snapshots(dir: &std::path::Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map_or(false, |x| x == "jpg"))
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort();
    let extra = files.len() - keep;
    for p in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(p);
    }
}

fn hint_of(f: FrameFormat) -> frame::Hint {
    #[allow(unreachable_patterns)]
    match f {
        FrameFormat::MJPEG => frame::Hint::Mjpeg,
        FrameFormat::YUYV => frame::Hint::Yuyv,
        FrameFormat::NV12 => frame::Hint::Nv12,
        FrameFormat::RAWRGB => frame::Hint::Rgb,
        FrameFormat::RAWBGR => frame::Hint::Bgr,
        FrameFormat::GRAY => frame::Hint::Gray,
        _ => frame::Hint::Unknown,
    }
}

/// Prefers about 640 pixels wide at 15 fps or better, which is plenty for
/// detection and light on the CPU.
fn pick_format(list: &[CameraFormat]) -> Option<CameraFormat> {
    let score = |f: &CameraFormat| -> i64 {
        let w = f.width() as i64;
        let fps = f.frame_rate() as i64;
        let fps_score = if fps >= 15 { 1000 + fps.min(30) * 10 } else { fps * 40 };
        let fmt_score = match f.format() {
            FrameFormat::MJPEG => 30,
            FrameFormat::NV12 => 25,
            FrameFormat::YUYV => 20,
            _ => 0,
        };
        fps_score + fmt_score - (w - 640).abs()
    };
    list.iter().filter(|f| FORMATS.contains(&f.format())).max_by_key(|f| score(f)).copied()
}

fn open_camera(index: u32) -> Result<nokhwa::Camera, String> {
    let req = RequestedFormat::with_formats(RequestedFormatType::AbsoluteHighestFrameRate, &FORMATS);
    let mut cam = nokhwa::Camera::new(CameraIndex::Index(index), req).map_err(|e| e.to_string())?;
    if let Ok(list) = cam.compatible_camera_formats() {
        if let Some(best) = pick_format(&list) {
            if best != cam.camera_format() {
                let req2 = RequestedFormat::with_formats(RequestedFormatType::Exact(best), &FORMATS);
                if let Err(e) = cam.set_camera_requset(req2) {
                    util::log(&format!("camera kept its default format ({})", e));
                }
            }
        }
    }
    cam.open_stream().map_err(|e| e.to_string())?;
    Ok(cam)
}

#[cfg(target_os = "macos")]
fn request_permission() -> bool {
    if nokhwa::nokhwa_check() {
        return true;
    }
    let (tx, rx) = mpsc::channel::<bool>();
    let tx = Mutex::new(tx);
    nokhwa::nokhwa_initialize(move |granted| {
        if let Ok(t) = tx.lock() {
            let _ = t.send(granted);
        }
    });
    rx.recv_timeout(Duration::from_secs(120)).unwrap_or(false) || nokhwa::nokhwa_check()
}
