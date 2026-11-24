//! Updates. Release builds know which GitHub repository they came from (the
//! release workflow sets ROOMGUARD_REPO at build time). The app asks GitHub
//! for the latest release, and when there's a newer version it shows the
//! release notes and asks before downloading anything.
//!
//! On Windows and Linux the new program replaces the old one in place and
//! starts on the next launch. On macOS the new disk image opens so you can
//! drag the app into Applications. Your settings and history live in a
//! separate data folder, so updating never touches them.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use serde::Deserialize;

use crate::util;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub asset_url: Option<String>,
    pub asset_name: String,
}

#[derive(Clone, Debug)]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { got: u64, total: u64 },
    Ready(String),
    Failed(String),
    NoRepo,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Clone)]
pub struct Updater {
    state: Arc<Mutex<UpdateState>>,
}

pub fn asset_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x86_64.exe"
    } else if cfg!(target_os = "macos") {
        "macos-universal.dmg"
    } else if cfg!(target_arch = "aarch64") {
        "linux-aarch64"
    } else {
        "linux-x86_64"
    }
}

/// True if `remote` is a newer version than `local` (major.minor.patch).
pub fn newer(remote: &str, local: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.trim_start_matches('v')
            .split(|c: char| c == '.' || c == '-' || c == '+')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .take(3)
            .collect()
    };
    let (a, b) = (parse(remote), parse(local));
    for i in 0..3 {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

fn agent(secs: u64) -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(secs)).build()
}

fn fetch_latest(repo: &str) -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let resp = agent(20)
        .get(&url)
        .set("User-Agent", "RoomGuard")
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::Status(404, _) => "no releases have been published yet".to_string(),
            other => other.to_string(),
        })?;
    let body = resp.into_string().map_err(|e| e.to_string())?;
    let gh: GhRelease = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let version = gh.tag_name.trim_start_matches('v').to_string();
    if !newer(&version, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let asset = gh.assets.iter().find(|a| a.name.ends_with(asset_suffix()));
    Ok(Some(Release {
        version,
        notes: gh.body.unwrap_or_default(),
        page: gh.html_url,
        asset_url: asset.map(|a| a.browser_download_url.clone()),
        asset_name: asset.map(|a| a.name.clone()).unwrap_or_default(),
    }))
}

impl Updater {
    pub fn new() -> Updater {
        Updater { state: Arc::new(Mutex::new(UpdateState::Idle)) }
    }

    pub fn repo() -> Option<&'static str> {
        option_env!("ROOMGUARD_REPO").filter(|r| r.contains('/') && !r.trim().is_empty())
    }

    pub fn state(&self) -> UpdateState {
        self.state.lock().map(|s| s.clone()).unwrap_or(UpdateState::Idle)
    }

    fn set(&self, s: UpdateState, ctx: &egui::Context) {
        if let Ok(mut g) = self.state.lock() {
            *g = s;
        }
        ctx.request_repaint();
    }

    pub fn check(&self, ctx: &egui::Context) {
        let Some(repo) = Self::repo() else {
            self.set(UpdateState::NoRepo, ctx);
            return;
        };
        self.set(UpdateState::Checking, ctx);
        let me = self.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || match fetch_latest(repo) {
            Ok(Some(rel)) => me.set(UpdateState::Available(rel), &ctx),
            Ok(None) => me.set(UpdateState::UpToDate, &ctx),
            Err(e) => me.set(UpdateState::Failed(e), &ctx),
        });
    }

    pub fn install(&self, rel: Release, ctx: &egui::Context) {
        let Some(url) = rel.asset_url.clone() else {
            let _ = open::that(&rel.page);
            self.set(UpdateState::Ready("The release page is open in your browser.".to_string()), ctx);
            return;
        };
        let me = self.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            me.set(UpdateState::Downloading { got: 0, total: 0 }, &ctx);
            match me.download_and_install(&url, &rel, &ctx) {
                Ok(msg) => me.set(UpdateState::Ready(msg), &ctx),
                Err(e) => me.set(UpdateState::Failed(e), &ctx),
            }
        });
    }

    fn download(&self, url: &str, dest: &Path, ctx: &egui::Context) -> Result<(), String> {
        let resp = agent(600).get(url).set("User-Agent", "RoomGuard").call().map_err(|e| e.to_string())?;
        let total: u64 = resp.header("Content-Length").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut reader = resp.into_reader();
        let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 64 * 1024];
        let mut got = 0u64;
        let mut last = Instant::now();
        loop {
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            got += n as u64;
            if last.elapsed() > Duration::from_millis(150) {
                last = Instant::now();
                self.set(UpdateState::Downloading { got, total }, ctx);
            }
        }
        file.sync_all().map_err(|e| e.to_string())?;
        if total > 0 && got != total {
            return Err("the download was cut short, please try again".to_string());
        }
        Ok(())
    }

    fn download_and_install(&self, url: &str, rel: &Release, ctx: &egui::Context) -> Result<String, String> {
        if cfg!(target_os = "macos") {
            let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
            let dest = dir.join(if rel.asset_name.is_empty() { "RoomGuard.dmg" } else { rel.asset_name.as_str() });
            self.download(url, &dest, ctx)?;
            let _ = open::that(&dest);
            return Ok("The new version's disk image is open. Drag RoomGuard into Applications to replace this one, then reopen it.".to_string());
        }
        let dest: PathBuf = util::data_dir().join(format!("roomguard-update-{}", rel.version));
        self.download(url, &dest, ctx)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        }
        self_replace::self_replace(&dest).map_err(|e| format!("couldn't replace the program: {}", e))?;
        let _ = std::fs::remove_file(&dest);
        util::log(&format!("updated to {}", rel.version));
        Ok(format!("Version {} is installed. Restart RoomGuard to start using it.", rel.version))
    }
}

/// Starts a fresh copy of the (possibly just updated) program.
pub fn relaunch() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
}
