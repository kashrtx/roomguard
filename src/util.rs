//! Small helpers shared across the app: time, hashing, random tokens, crash
//! safe file writes and the log file.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chrono::{Datelike, TimeZone, Timelike};

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);
static BACKED_UP: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Thread priorities, so the keypad, its sound and the window always win the
/// CPU over heavy background work like body tracking. Windows only; on macOS
/// and Linux the scheduler balances these threads well on its own.
pub mod prio {
    pub const BACKGROUND: i32 = -1;
    pub const PANEL: i32 = 1;
    pub const AUDIO: i32 = 2;
}

#[cfg(windows)]
pub fn set_thread_priority(level: i32) {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThread() -> isize;
        fn SetThreadPriority(thread: isize, priority: i32) -> i32;
    }
    // SAFETY: GetCurrentThread returns a pseudo handle for the calling
    // thread that needs no closing; SetThreadPriority only reads it.
    unsafe {
        SetThreadPriority(GetCurrentThread(), level);
    }
}

#[cfg(not(windows))]
pub fn set_thread_priority(_level: i32) {}

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

/// Saving happens on one background thread, so a slow disk (antivirus
/// scanning every write, a synced folder, a drive waking up) can never stall
/// the keypad, the panel clock or the camera. A file saved several times in a
/// row is written once with its newest contents, a load sees a save the
/// moment it's queued, and anything still waiting is written before exit.
struct SaveQueue {
    state: Mutex<SaveState>,
    wake: Condvar,
    idle: Condvar,
    threaded: AtomicBool,
}

#[derive(Default)]
struct SaveState {
    pending: Vec<(PathBuf, Arc<Vec<u8>>)>,
    writing: Option<(PathBuf, Arc<Vec<u8>>)>,
}

static SAVES: OnceLock<&'static SaveQueue> = OnceLock::new();

fn lock_saves(q: &SaveQueue) -> MutexGuard<'_, SaveState> {
    q.state.lock().unwrap_or_else(|p| p.into_inner())
}

fn save_queue() -> &'static SaveQueue {
    SAVES.get_or_init(|| {
        let q: &'static SaveQueue = Box::leak(Box::new(SaveQueue {
            state: Mutex::new(SaveState::default()),
            wake: Condvar::new(),
            idle: Condvar::new(),
            threaded: AtomicBool::new(false),
        }));
        match std::thread::Builder::new().name("saver".into()).spawn(move || save_worker(q)) {
            Ok(_) => q.threaded.store(true, Ordering::SeqCst),
            Err(e) => log(&format!("could not start the save thread, saving directly instead: {}", e)),
        }
        q
    })
}

fn save_worker(q: &'static SaveQueue) {
    set_thread_priority(prio::BACKGROUND);
    loop {
        let (path, bytes) = {
            let mut st = lock_saves(q);
            while st.pending.is_empty() {
                st = q.wake.wait(st).unwrap_or_else(|p| p.into_inner());
            }
            let (path, bytes) = st.pending.remove(0);
            st.writing = Some((path.clone(), bytes.clone()));
            (path, bytes)
        };
        write_now(&path, &bytes);
        let mut st = lock_saves(q);
        st.writing = None;
        if st.pending.is_empty() {
            q.idle.notify_all();
        }
    }
}

/// The newest bytes queued for `path` that may not be on disk yet.
fn queued_bytes(path: &Path) -> Option<Arc<Vec<u8>>> {
    let q = SAVES.get().copied()?;
    let st = lock_saves(q);
    if let Some(slot) = st.pending.iter().find(|slot| slot.0.as_path() == path) {
        return Some(slot.1.clone());
    }
    st.writing.as_ref().filter(|w| w.0.as_path() == path).map(|w| w.1.clone())
}

/// Waits until every queued save is on disk, for at most `max`. Returns
/// false if time ran out first.
pub fn flush_saves(max: Duration) -> bool {
    let Some(q) = SAVES.get().copied() else {
        return true;
    };
    let deadline = Instant::now() + max;
    let mut st = lock_saves(q);
    while !st.pending.is_empty() || st.writing.is_some() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return false;
        }
        st = match q.idle.wait_timeout(st, left) {
            Ok((g, _)) => g,
            Err(p) => p.into_inner().0,
        };
    }
    true
}

pub fn save_json<T: serde::Serialize>(path: &Path, value: &T) -> bool {
    let bytes = match serde_json::to_vec_pretty(value) {
        Ok(b) => b,
        Err(e) => {
            log(&format!("could not encode {}: {}", path.display(), e));
            return false;
        }
    };
    let q = save_queue();
    if !q.threaded.load(Ordering::SeqCst) {
        return write_now(path, &bytes);
    }
    let bytes = Arc::new(bytes);
    let mut st = lock_saves(q);
    if let Some(slot) = st.pending.iter_mut().find(|slot| slot.0.as_path() == path) {
        slot.1 = bytes;
    } else {
        st.pending.push((path.to_path_buf(), bytes));
    }
    q.wake.notify_one();
    true
}

fn write_now(path: &Path, bytes: &[u8]) -> bool {
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
    match write_atomic(path, bytes) {
        Ok(()) => true,
        Err(e) => {
            log(&format!("could not save {}: {}", path.display(), e));
            false
        }
    }
}

/// Deletes everything RoomGuard has saved, except its log, for a fresh
/// start. Anything still queued is written first so nothing lands after.
pub fn wipe_data() {
    flush_saves(Duration::from_secs(4));
    let dir = data_dir();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("roomguard") && name.ends_with(".log") {
                continue;
            }
            let p = e.path();
            let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
        }
    }
    if let Ok(mut list) = BACKED_UP.lock() {
        list.clear();
    }
    log("everything saved was erased for a fresh start");
}

pub fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    // A save that's queued but not written yet is the newest copy.
    if let Some(bytes) = queued_bytes(path) {
        if let Ok(v) = serde_json::from_slice::<T>(&bytes) {
            return Some(v);
        }
    }
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

/// Day of week index matching VISTA-20P SMTWTFS order (0=Sun .. 6=Sat).
pub fn weekday_idx(ms: i64) -> u8 {
    local(ms).weekday().num_days_from_sunday() as u8
}

/// 3-letter uppercase weekday matching the 6160V TIME/DATE display.
pub fn weekday_abbr(ms: i64) -> &'static str {
    match weekday_idx(ms) {
        0 => "SUN",
        1 => "MON",
        2 => "TUE",
        3 => "WED",
        4 => "THU",
        5 => "FRI",
        _ => "SAT",
    }
}

/// Authentic VISTA-20P 6160V Time/Date display lines:
/// Line 1: `TIME/DATE   THU`
/// Line 2: `10:12PM 07/11/19`
pub fn fmt_vista_time_date(ms: i64) -> (String, String) {
    let p = parts(ms);
    let (h12, ampm) = match p.hour {
        0 => (12, "AM"),
        1..=11 => (p.hour, "AM"),
        12 => (12, "PM"),
        _ => (p.hour - 12, "PM"),
    };
    let l1 = format!("TIME/DATE   {}", weekday_abbr(ms));
    let l2 = format!("{:02}:{:02}{} {:02}/{:02}/{:02}", h12, p.minute, ampm, p.month, p.day, p.year.rem_euclid(100));
    (l1, l2)
}

/// Authentic VISTA-20P `#60` Event Log timestamp line: `12:34AM 01/02/00`.
pub fn fmt_vista_event_time(ms: i64) -> String {
    fmt_vista_time_date(ms).1
}

/// How far to shift the panel clock so it reads the exact local date and time right now.
/// Returns `None` if the calendar date or time is invalid (e.g. Feb 30).
pub fn offset_to_datetime(now_ms: i64, year: i32, month: u32, day: u32, hour24: u32, minute: u32) -> Option<i64> {
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let dt = date.and_hms_opt(hour24, minute, 0)?;
    let local_dt = chrono::Local.from_local_datetime(&dt).earliest()?;
    Some(local_dt.timestamp_millis() - now_ms)
}
