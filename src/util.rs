//! Small helpers shared across the app: time, hashing, random tokens, crash
//! safe file writes and the log file.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Timelike};

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);
static BACKED_UP: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Where everything RoomGuard saves lives. Deleting or updating the app
/// never touches this folder.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ROOMGUARD_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("RoomGuard")
}

pub fn log_init() {
    let dir = data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("roomguard.log");
    if std::fs::metadata(&path).map(|m| m.len() > 2_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("roomguard.old.log"));
    }
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        if let Ok(mut guard) = LOG.lock() {
            *guard = Some(file);
        }
    }
    std::panic::set_hook(Box::new(|info| {
        log(&format!("panic: {}", info));
    }));
    log(&format!(
        "RoomGuard {} starting on {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    ));
}

pub fn log(msg: &str) {
    let line = format!("{} {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    let _ = std::io::stderr().write_all(line.as_bytes());
    if let Ok(mut guard) = LOG.lock() {
        if let Some(file) = guard.as_mut() {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

pub fn random_u64() -> u64 {
    let mut h = RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    h.write_u128(nanos);
    h.write_u32(std::process::id());
    h.finish()
}

/// A random token using letters and digits that can't be confused for each
/// other when read aloud or typed on a phone.
pub fn random_token(len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut out = String::with_capacity(len);
    while out.len() < len {
        let mut v = random_u64();
        for _ in 0..10 {
            if out.len() >= len {
                break;
            }
            out.push(ALPHABET[(v % ALPHABET.len() as u64) as usize] as char);
            v /= ALPHABET.len() as u64;
        }
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(data);
    let mut s = String::with_capacity(64);
    for b in digest.iter() {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

/// Codes and passcodes are only ever stored salted and hashed.
pub fn hash_secret(salt: &str, secret: &str) -> String {
    sha256_hex(format!("roomguard|{}|{}", salt, secret).as_bytes())
}

pub fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let n = ((chunk[0] as u32) << 16) | ((b1 as u32) << 8) | b2 as u32;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { T[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[(n & 63) as usize] as char } else { '=' });
    }
    out
}

pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let t = out.trim_matches('-').to_string();
    if t.is_empty() {
        "camera".to_string()
    } else {
        t
    }
}

/// Writes to a temporary file first and renames it into place, so a crash or
/// power cut in the middle of a save can never leave a half written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub fn save_json<T: serde::Serialize>(path: &Path, value: &T) -> bool {
    let bytes = match serde_json::to_vec_pretty(value) {
        Ok(b) => b,
        Err(e) => {
            log(&format!("could not encode {}: {}", path.display(), e));
            return false;
        }
    };
    // Once per run, keep the last known good copy as a backup.
    let first_save = BACKED_UP
        .lock()
        .map(|mut list| {
            if list.iter().any(|p| p == path) {
                false
            } else {
                list.push(path.to_path_buf());
                true
            }
        })
        .unwrap_or(false);
    if first_save && path.exists() {
        let _ = std::fs::copy(path, path.with_extension("bak"));
    }
    match write_atomic(path, &bytes) {
        Ok(()) => true,
        Err(e) => {
            log(&format!("could not save {}: {}", path.display(), e));
            false
        }
    }
}

pub fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    for candidate in [path.to_path_buf(), path.with_extension("bak")] {
        let Ok(bytes) = std::fs::read(&candidate) else {
            continue;
        };
        match serde_json::from_slice::<T>(&bytes) {
            Ok(v) => return Some(v),
            Err(e) => {
                log(&format!("{} could not be read ({}), trying the backup", candidate.display(), e));
                let keep = candidate.with_extension(format!("corrupt-{}", now_ms()));
                let _ = std::fs::copy(&candidate, keep);
            }
        }
    }
    None
}

pub fn local(ms: i64) -> chrono::DateTime<chrono::Local> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local))
        .unwrap_or_else(chrono::Local::now)
}

pub fn fmt_clock(ms: i64) -> String {
    local(ms).format("%-I:%M %p").to_string()
}

pub fn fmt_stamp(ms: i64) -> String {
    local(ms).format("%b %-d, %-I:%M:%S %p").to_string()
}

/// The time alone for today, otherwise the date and time.
pub fn fmt_when(ms: i64) -> String {
    let d = local(ms);
    if d.date_naive() == chrono::Local::now().date_naive() {
        d.format("%-I:%M %p").to_string()
    } else {
        d.format("%b %-d, %-I:%M %p").to_string()
    }
}

pub fn file_stamp(ms: i64) -> String {
    local(ms).format("%Y-%m-%d_%H-%M-%S").to_string()
}

#[derive(Clone, Copy, Debug)]
pub struct LocalParts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub ordinal: u32,
}

pub fn parts(ms: i64) -> LocalParts {
    let d = local(ms);
    LocalParts {
        year: d.year(),
        month: d.month(),
        day: d.day(),
        hour: d.hour(),
        minute: d.minute(),
        second: d.second(),
        ordinal: d.ordinal(),
    }
}

/// How far to shift the panel clock so it reads `hour:minute` right now.
pub fn offset_to_time(now_ms: i64, hour: u32, minute: u32) -> i64 {
    let p = parts(now_ms);
    let cur = (p.hour * 3600 + p.minute * 60 + p.second) as i64;
    let want = (hour * 3600 + minute * 60) as i64;
    let mut d = want - cur;
    if d > 43_200 {
        d -= 86_400;
    }
    if d < -43_200 {
        d += 86_400;
    }
    d * 1000
}
