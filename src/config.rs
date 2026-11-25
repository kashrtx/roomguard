//! Everything the user can set in the app. Saved as JSON in the data folder.
//! Every struct uses `serde(default)`, so settings saved by an older version
//! always load in a newer one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::engine::types::{Model, SirenModel};
use crate::util;

/// A rectangle in normalized coordinates (0 to 1 across the view or map).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct NRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl NRect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        NRect { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
    pub fn from_points(ax: f32, ay: f32, bx: f32, by: f32) -> Self {
        let x0 = ax.min(bx).clamp(0.0, 1.0);
        let y0 = ay.min(by).clamp(0.0, 1.0);
        let x1 = ax.max(bx).clamp(0.0, 1.0);
        let y1 = ay.max(by).clamp(0.0, 1.0);
        NRect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
    }
    pub fn overlap(&self, o: &NRect) -> f32 {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        ((x1 - x0).max(0.0) * (y1 - y0).max(0.0)) / (self.w * self.h).max(1e-6)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Lens {
    #[default]
    Normal,
    Wide,
    Fisheye,
}

impl Lens {
    /// Typical horizontal field of view in degrees.
    pub fn hfov(self) -> f32 {
        match self {
            Lens::Normal => 68.0,
            Lens::Wide => 105.0,
            Lens::Fisheye => 150.0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Lens::Normal => "Normal webcam",
            Lens::Wide => "Wide angle",
            Lens::Fisheye => "Fisheye",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PtzKind {
    #[default]
    Fixed,
    Uvc,
    Obsbot,
}

impl PtzKind {
    pub fn label(self) -> &'static str {
        match self {
            PtzKind::Fixed => "Fixed camera",
            PtzKind::Uvc => "Pan and tilt webcam (USB PTZ)",
            PtzKind::Obsbot => "OBSBOT through OBSBOT Center",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PtzCfg {
    pub kind: PtzKind,
    pub follow: bool,
    pub obsbot_host: String,
    pub obsbot_port: u16,
    pub obsbot_four_args: bool,
    pub invert_pan: bool,
    pub invert_tilt: bool,
    pub home_pan: f32,
    pub home_tilt: f32,
    pub search_secs: f32,
}

impl Default for PtzCfg {
    fn default() -> Self {
        PtzCfg {
            kind: PtzKind::Fixed,
            follow: true,
            obsbot_host: "127.0.0.1".to_string(),
            obsbot_port: 16284,
            obsbot_four_args: false,
            invert_pan: false,
            invert_tilt: false,
            home_pan: 0.0,
            home_tilt: 0.0,
            search_secs: 20.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraCfg {
    pub enabled: bool,
    pub device: u32,
    pub device_name: String,
    pub mirror: bool,
    pub lens: Lens,
    /// 1 (least sensitive) to 5 (most).
    pub sensitivity: u8,
    pub doorway: Option<NRect>,
    pub ignore: Vec<NRect>,
    /// Treat the doorway like a door contact: a door left open trips it too.
    pub door_contact: bool,
    pub faces: bool,
    pub face_disarm: bool,
    pub liveness: bool,
    /// 1 relaxed, 2 balanced, 3 strict.
    pub strictness: u8,
    pub auto_frame: bool,
    pub learn_map: bool,
    pub ptz: PtzCfg,
}

impl Default for CameraCfg {
    fn default() -> Self {
        CameraCfg {
            enabled: true,
            device: 0,
            device_name: String::new(),
            mirror: true,
            lens: Lens::Normal,
            sensitivity: 3,
            doorway: None,
            ignore: Vec::new(),
            door_contact: true,
            faces: true,
            face_disarm: true,
            liveness: true,
            strictness: 2,
            auto_frame: false,
            learn_map: true,
            ptz: PtzCfg::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundCfg {
    pub master: f32,
    pub siren: f32,
    pub keypad: f32,
    pub siren_model: SirenModel,
    /// Wire the WAVE-2F to its warble terminal instead of the steady one.
    pub wave2_warble: bool,
    /// Play the communicator's phone line tones while it reports.
    pub line_audio: bool,
}

impl Default for SoundCfg {
    fn default() -> Self {
        SoundCfg {
            master: 0.8,
            siren: 0.45,
            keypad: 0.7,
            siren_model: SirenModel::Wave2F,
            wave2_warble: true,
            line_audio: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OperatorCfg {
    /// Simulated central station that chats and calls on screen.
    pub enabled: bool,
    pub operator_name: String,
    pub passcode_salt: String,
    pub passcode_hash: String,
    /// Real push notifications to your phone through ntfy.
    pub phone_alerts: bool,
    pub ntfy_server: String,
    pub ntfy_topic: String,
    pub ntfy_token: String,
    pub send_photos: bool,
    pub heads_up_on_entry: bool,
    pub open_close_reports: bool,
    pub trouble_reports: bool,
}

impl Default for OperatorCfg {
    fn default() -> Self {
        OperatorCfg {
            enabled: true,
            operator_name: "Dana".to_string(),
            passcode_salt: String::new(),
            passcode_hash: String::new(),
            phone_alerts: false,
            ntfy_server: "https://ntfy.sh".to_string(),
            ntfy_topic: format!("roomguard-{}", util::random_token(14)),
            ntfy_token: String::new(),
            send_photos: true,
            heads_up_on_entry: false,
            open_close_reports: false,
            trouble_reports: true,
        }
    }
}

impl OperatorCfg {
    pub fn set_passcode(&mut self, word: &str) {
        self.passcode_salt = util::random_token(12);
        self.passcode_hash = util::hash_secret(&self.passcode_salt, &word.trim().to_lowercase());
    }
    pub fn passcode_ok(&self, word: &str) -> bool {
        !self.passcode_hash.is_empty()
            && util::hash_secret(&self.passcode_salt, &word.trim().to_lowercase()) == self.passcode_hash
    }
    pub fn has_passcode(&self) -> bool {
        !self.passcode_hash.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiCfg {
    pub lights_on: bool,
    pub door_open_1550: bool,
    pub door_open_5500: bool,
    pub door_open_6160: bool,
    pub right_tab: u8,
    pub show_motion: bool,
    pub show_tracks: bool,
    /// The hidden installer tab in Settings has been unlocked.
    pub installer_view: bool,
    /// Draw with OpenGL instead of Direct3D, Vulkan or Metal.
    pub opengl: bool,
}

impl Default for UiCfg {
    fn default() -> Self {
        UiCfg {
            lights_on: true,
            door_open_1550: true,
            door_open_5500: true,
            door_open_6160: true,
            right_tab: 0,
            show_motion: true,
            show_tracks: true,
            installer_view: false,
            opengl: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub setup_done: bool,
    pub admin_name: String,
    pub model: Model,
    pub camera: CameraCfg,
    pub sound: SoundCfg,
    pub operator: OperatorCfg,
    pub ui: UiCfg,
    pub labels_1550: Vec<String>,
    pub labels_1832: Vec<String>,
    pub labels_vista: Vec<String>,
    pub check_updates: bool,
    pub skipped_version: String,
    /// Closing the app while armed needs your code.
    pub close_needs_code: bool,
    /// Stop the computer from sleeping while armed.
    pub keep_awake: bool,
    /// Open RoomGuard when you sign in.
    pub autostart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            setup_done: false,
            admin_name: String::new(),
            model: Model::Pc1832,
            camera: CameraCfg::default(),
            sound: SoundCfg::default(),
            operator: OperatorCfg::default(),
            ui: UiCfg::default(),
            labels_1550: default_labels(Model::Pc1550),
            labels_1832: default_labels(Model::Pc1832),
            labels_vista: default_labels(Model::Vista20p),
            check_updates: true,
            skipped_version: String::new(),
            close_needs_code: true,
            keep_awake: true,
            autostart: false,
        }
    }
}

pub fn default_labels(model: Model) -> Vec<String> {
    let names: &[&str] = match model {
        Model::Pc1550 => &["Room Doorway", "Room Motion", "Window", "Front Door", "Hall Motion", "Smoke Detector"],
        Model::Pc1832 => &[
            "Room Doorway",
            "Room Motion",
            "Window",
            "Front Door",
            "Hall Motion",
            "Smoke Detector",
            "Garage Door",
            "Basement",
        ],
        Model::Vista20p => &[
            "Room Door",
            "Room Motion",
            "Window",
            "Front Door",
            "Hall Motion",
            "Smoke Detector",
            "Garage Door",
            "Basement",
        ],
    };
    names.iter().map(|s| s.to_string()).collect()
}

impl AppConfig {
    pub fn path() -> PathBuf {
        util::data_dir().join("config.json")
    }

    pub fn load() -> AppConfig {
        let mut cfg = util::load_json::<AppConfig>(&Self::path()).unwrap_or_default();
        for model in [Model::Pc1550, Model::Pc1832, Model::Vista20p] {
            let want = model.zone_count();
            let defaults = default_labels(model);
            let labels = cfg.labels_mut(model);
            while labels.len() < want {
                labels.push(defaults[labels.len()].clone());
            }
            labels.truncate(want);
        }
        cfg.camera.sensitivity = cfg.camera.sensitivity.clamp(1, 5);
        cfg.camera.strictness = cfg.camera.strictness.clamp(1, 3);
        cfg
    }

    pub fn save(&self) -> bool {
        util::save_json(&Self::path(), self)
    }

    pub fn labels(&self, model: Model) -> &Vec<String> {
        match model {
            Model::Pc1550 => &self.labels_1550,
            Model::Pc1832 => &self.labels_1832,
            Model::Vista20p => &self.labels_vista,
        }
    }

    pub fn labels_mut(&mut self, model: Model) -> &mut Vec<String> {
        match model {
            Model::Pc1550 => &mut self.labels_1550,
            Model::Pc1832 => &mut self.labels_1832,
            Model::Vista20p => &mut self.labels_vista,
        }
    }

    pub fn display_name(&self) -> String {
        let n = self.admin_name.trim();
        if n.is_empty() {
            "you".to_string()
        } else {
            n.to_string()
        }
    }
}
