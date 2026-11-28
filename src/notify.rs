//! Phone alerts through ntfy (ntfy.sh, or your own ntfy server). The free
//! ntfy app subscribes to a private topic name, and RoomGuard publishes to it
//! with a plain HTTP request, attaching the camera snapshot when there is
//! one. A failed alert is retried, eight attempts in all, the same number of
//! dialing attempts a PC1550 makes before it shows a communication trouble.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use crate::config::OperatorCfg;
use crate::engine::types::{AlarmKind, Report, ReportKind};
use crate::engine::EngineMsg;
use crate::util;

#[derive(Clone, Debug)]
pub struct NotifyJob {
    pub title: String,
    pub message: String,
    pub priority: u8,
    pub tags: String,
    pub photo: Option<PathBuf>,
    pub test: bool,
}

/// Returns the alert and whether it can use a fresh event snapshot.
pub fn for_report(report: &Report, cfg: &OperatorCfg) -> Option<(NotifyJob, bool)> {
    if !cfg.phone_alerts {
        return None;
    }
    let zone = report.zone_label.trim();
    let location = if !zone.is_empty() {
        zone.to_string()
    } else if report.zone_or_user != 0 {
        format!("Zone {:02}", report.zone_or_user)
    } else {
        "Keypad".to_string()
    };
    let at = util::fmt_clock(report.ts);
    let (title, message, priority, tag, snapshot) = match report.kind {
        ReportKind::Alarm(AlarmKind::Duress) => (
            "Duress code used".into(), format!("The duress code was entered at {at}."), 5, "warning", false,
        ),
        ReportKind::Alarm(kind) => (
            format!("{} alarm: {location}", kind.label()), format!("Alarm triggered at {at}."),
            5, if kind == AlarmKind::Fire { "fire" } else { "rotating_light" }, true,
        ),
        ReportKind::EntryStarted if cfg.heads_up_on_entry => (
            format!("{location} opened"), format!("Entry countdown started at {at}."), 3, "door", true,
        ),
        ReportKind::PhysicalEntry if cfg.heads_up_on_entry => (
            "Physical panel entry delay".into(), format!("The panel reported an entry delay at {at}."), 3, "door", true,
        ),
        ReportKind::PhysicalAlarm => (
            "Physical panel alarm".into(), format!("The panel reported an alarm at {at}."), 5, "rotating_light", true,
        ),
        ReportKind::Stranger if cfg.unknown_person_alerts => (
            "Person detected while armed".into(), format!("RoomGuard saw someone it could not identify at {at}."), 4, "bust_in_silhouette", true,
        ),
        ReportKind::Lockout => (
            "Keypad locked".into(), format!("Several incorrect codes were entered at {at}."), 4, "lock", true,
        ),
        // A phone delivery failure must never queue another phone alert about itself.
        // Filter non-trouble reports that the simulated panel categorizes as restores.
        ReportKind::Trouble | ReportKind::Restore if cfg.trouble_reports && matches!(report.cid, 300..=399) && report.cid != 354 => (
            if report.kind == ReportKind::Trouble { "RoomGuard needs attention".into() } else { "RoomGuard recovered".into() },
            format!("{} at {at}.", report.text), 3, "warning", false,
        ),
        ReportKind::Opening | ReportKind::Closing if cfg.open_close_reports => (
            if report.kind == ReportKind::Opening { "RoomGuard disarmed".into() } else { "RoomGuard armed".into() },
            format!("{} at {at}.", report.text), 2, "lock", false,
        ),
        ReportKind::PowerUp => (
            "RoomGuard restarted".into(), format!("RoomGuard came back armed at {at}."), 3, "electric_plug", false,
        ),
        _ => return None,
    };
    Some((NotifyJob { title, message, priority, tags: tag.into(), photo: None, test: false }, snapshot))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NtfyCfg {
    pub enabled: bool,
    pub server: String,
    pub topic: String,
    pub token: String,
    pub photos: bool,
}

impl NtfyCfg {
    pub fn from(op: &OperatorCfg) -> NtfyCfg {
        NtfyCfg {
            enabled: op.phone_alerts,
            server: op.ntfy_server.trim().trim_end_matches('/').to_string(),
            topic: op.ntfy_topic.trim().to_string(),
            token: op.ntfy_token.trim().to_string(),
            photos: op.send_photos,
        }
    }

    fn server(&self) -> &str {
        if self.server.is_empty() {
            "https://ntfy.sh"
        } else {
            &self.server
        }
    }

    pub fn topic_url(&self) -> String {
        format!("{}/{}", self.server(), self.topic)
    }
}

enum Cmd {
    Send(NotifyJob),
    Config(NtfyCfg),
}

#[derive(Clone)]
pub struct NotifyHandle {
    tx: Sender<Cmd>,
}

impl NotifyHandle {
    pub fn send(&self, job: NotifyJob) {
        let _ = self.tx.send(Cmd::Send(job));
    }
    pub fn config(&self, op: &OperatorCfg) {
        let _ = self.tx.send(Cmd::Config(NtfyCfg::from(op)));
    }
}

pub fn start(engine: Sender<EngineMsg>) -> NotifyHandle {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new().name("notify".into()).spawn(move || run(rx, engine));
    if let Err(e) = spawned {
        util::log(&format!("could not start the notification thread: {}", e));
    }
    NotifyHandle { tx }
}

fn run(rx: Receiver<Cmd>, engine: Sender<EngineMsg>) {
    let mut cfg = NtfyCfg::default();
    let mut queue: Vec<(NotifyJob, u32, Instant)> = Vec::new();
    loop {
        let now = Instant::now();
        let wait = queue.iter().map(|(_, _, due)| due.saturating_duration_since(now))
            .min().unwrap_or(Duration::from_secs(60));
        match rx.recv_timeout(wait) {
            Ok(Cmd::Config(c)) => {
                if !c.enabled || (c.server.as_str(), c.topic.as_str(), c.token.as_str()) != (cfg.server.as_str(), cfg.topic.as_str(), cfg.token.as_str()) {
                    queue.clear();
                }
                cfg = c;
            }
            Ok(Cmd::Send(job)) if cfg.enabled || job.test => queue.push((job, 0, Instant::now())),
            Ok(Cmd::Send(_)) => {}
            Err(RecvTimeoutError::Timeout) if queue.is_empty() && cfg.enabled => {
                let _ = engine.send(EngineMsg::NetStatus(health(&cfg)));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let next = queue.iter().enumerate()
            .filter(|(_, (_, _, due))| *due <= now)
            .max_by_key(|(_, (job, _, due))| (job.priority, std::cmp::Reverse(*due)))
            .map(|(index, _)| index);
        if let Some(index) = next {
            let (job, tries, _) = queue.remove(index);
            let test = job.test;
            match send_now(&cfg, &job) {
                Ok(_) => {
                    let _ = engine.send(EngineMsg::NotifyResult { ok: true, test, err: String::new() });
                }
                Err(err) if test || tries + 1 >= 8 => {
                    let _ = engine.send(EngineMsg::NotifyResult { ok: false, test, err });
                }
                Err(err) => {
                    util::log(&format!("phone alert attempt {} failed: {}", tries + 1, err));
                    queue.push((job, tries + 1, Instant::now() + Duration::from_secs(6)));
                }
            }
        }
    }
}

/// HTTP headers must be plain ASCII. ntfy accepts RFC 2047 encoded UTF-8.
fn header_safe(s: &str) -> String {
    let flat = s.replace('\n', "\\n");
    if flat.is_ascii() {
        flat
    } else {
        format!("=?UTF-8?B?{}?=", util::base64(flat.as_bytes()))
    }
}

pub fn send_now(cfg: &NtfyCfg, job: &NotifyJob) -> Result<(), String> {
    send_now_timeout(cfg, job, Duration::from_secs(15))
}

pub fn send_now_timeout(cfg: &NtfyCfg, job: &NotifyJob, timeout: Duration) -> Result<(), String> {
    if cfg.topic.is_empty() {
        return Err("no topic is set".to_string());
    }
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let mut req = agent
        .put(&cfg.topic_url())
        .set("Title", &header_safe(&job.title))
        .set("Priority", &job.priority.clamp(1, 5).to_string());
    if !job.tags.is_empty() {
        req = req.set("Tags", &job.tags);
    }
    if !cfg.token.is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", cfg.token));
    }
    let photo = if cfg.photos { job.photo.as_ref().and_then(|p| std::fs::read(p).ok()) } else { None };
    let attached = photo.is_some();
    let result = match photo {
        Some(bytes) => {
            let name = job
                .photo
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "snapshot.jpg".to_string());
            let content_type = if name.ends_with(".gif") { "image/gif" } else { "image/jpeg" };
            req.set("Filename", &name)
                .set("Content-Type", content_type)
                .set("Message", &header_safe(&job.message))
                .send_bytes(&bytes)
        }
        None => req.send_string(&job.message),
    };
    match result {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(413 | 415, _)) if attached => {
            let mut text = job.clone();
            text.photo = None;
            send_now_timeout(cfg, &text, timeout)
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            Err(format!("the server answered {}: {}", code, body.chars().take(140).collect::<String>()))
        }
        Err(e) => Err(e.to_string()),
    }
}

fn health(cfg: &NtfyCfg) -> bool {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(8))
        .build()
        .get(&format!("{}/v1/health", cfg.server()))
        .call()
        .is_ok()
}
