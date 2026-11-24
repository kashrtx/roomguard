//! Phone alerts through ntfy (ntfy.sh, or your own ntfy server). The free
//! ntfy app subscribes to a private topic name, and RoomGuard publishes to it
//! with a plain HTTP request, attaching the camera snapshot when there is
//! one. A failed alert is retried, eight attempts in all, the same number of
//! dialing attempts a PC1550 makes before it shows a communication trouble.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::config::OperatorCfg;
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
    let mut queue: Vec<(NotifyJob, u32)> = Vec::new();
    loop {
        let wait = if queue.is_empty() { Duration::from_secs(60) } else { Duration::from_secs(6) };
        match rx.recv_timeout(wait) {
            Ok(Cmd::Config(c)) => cfg = c,
            Ok(Cmd::Send(job)) => queue.push((job, 0)),
            Err(RecvTimeoutError::Timeout) => {
                if queue.is_empty() && cfg.enabled {
                    let ok = health(&cfg);
                    let _ = engine.send(EngineMsg::NetStatus(ok));
                }
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let mut keep = Vec::new();
        for (job, tries) in queue.drain(..) {
            if !cfg.enabled && !job.test {
                continue;
            }
            match send_now(&cfg, &job) {
                Ok(()) => {
                    let _ = engine.send(EngineMsg::NotifyResult { ok: true, test: job.test, err: String::new() });
                }
                Err(e) => {
                    util::log(&format!("phone alert attempt {} failed: {}", tries + 1, e));
                    if job.test || tries + 1 >= 8 {
                        let _ = engine.send(EngineMsg::NotifyResult { ok: false, test: job.test, err: e });
                    } else {
                        keep.push((job, tries + 1));
                    }
                }
            }
        }
        queue = keep;
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
    if cfg.topic.is_empty() {
        return Err("no topic is set".to_string());
    }
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(15)).build();
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
    let result = match photo {
        Some(bytes) => {
            let name = job
                .photo
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "snapshot.jpg".to_string());
            req.set("Filename", &name).set("Message", &header_safe(&job.message)).send_bytes(&bytes)
        }
        None => req.send_string(&job.message),
    };
    match result {
        Ok(_) => Ok(()),
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
