//! Only one RoomGuard runs at a time. Two copies would fight over the camera,
//! double every beep and siren, and save over each other's panel.
//!
//! The first copy listens on a private port that only this computer can
//! reach (127.0.0.1, so no firewall prompt). The operating system frees it
//! the moment that copy ends, even after a crash. A second copy finds it
//! there, brings the open window to the front and quits.
//!
//! Updates and "Start over" restart RoomGuard by launching a new copy just
//! before the old one closes. That copy is started with `--restarted`, so it
//! waits for the old one to let go instead of quitting.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

use crate::util;

const PORT: u16 = 47_862;
const HELLO: &str = "roomguard-hello";

pub enum Start {
    /// This is the only copy running.
    First(Guard),
    /// Another copy is running and was brought to the front.
    AlreadyRunning,
}

/// Held for as long as this copy runs.
#[derive(Clone)]
pub struct Guard {
    ctx: Arc<Mutex<Option<egui::Context>>>,
}

impl Guard {
    /// Lets the guard bring this window to the front when another copy starts.
    pub fn attach(&self, ctx: &egui::Context) {
        if let Ok(mut g) = self.ctx.lock() {
            *g = Some(ctx.clone());
        }
    }
}

fn addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], PORT))
}

/// Becomes the one running copy, or finds the one that is. `wait` gives a
/// copy that was just restarted time for the old one to finish closing.
pub fn acquire(wait: bool) -> Start {
    let deadline = Instant::now() + Duration::from_secs(if wait { 15 } else { 0 });
    loop {
        if let Ok(listener) = TcpListener::bind(addr()) {
            let ctx = Arc::new(Mutex::new(None));
            let shared = ctx.clone();
            let spawned = std::thread::Builder::new().name("single-instance".into()).spawn(move || serve(listener, shared));
            if let Err(e) = spawned {
                util::log(&format!("couldn't start the single instance check: {}", e));
            }
            return Start::First(Guard { ctx });
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    match ask_running() {
        Some(pid) => {
            util::log(&format!("RoomGuard is already open (process {}), bringing it to the front", pid));
            raise_window_of(pid);
            Start::AlreadyRunning
        }
        None => {
            // Something that isn't RoomGuard holds the port. Don't refuse to start over it.
            util::log("another program is using RoomGuard's single instance port, so the check is off this time");
            Start::First(Guard { ctx: Arc::new(Mutex::new(None)) })
        }
    }
}

/// Reads one line, up to 64 bytes, without the newline.
fn read_line(s: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while buf.len() < 64 {
        match s.read(&mut byte) {
            Ok(1) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            _ => break,
        }
    }
    String::from_utf8_lossy(&buf).trim().to_string()
}

/// Asks the copy that holds the port who it is. Its process id if it's RoomGuard.
fn ask_running() -> Option<u32> {
    let mut s = TcpStream::connect_timeout(&addr(), Duration::from_millis(800)).ok()?;
    let _ = s.set_read_timeout(Some(Duration::from_secs(4)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
    s.write_all(format!("{}\n", HELLO).as_bytes()).ok()?;
    let reply = read_line(&mut s);
    reply.strip_prefix("roomguard ")?.trim().parse::<u32>().ok()
}

/// Answers later copies, and comes to the front when one starts.
fn serve(listener: TcpListener, ctx: Arc<Mutex<Option<egui::Context>>>) {
    for conn in listener.incoming() {
        let Ok(mut s) = conn else {
            continue;
        };
        let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = s.set_write_timeout(Some(Duration::from_secs(1)));
        if read_line(&mut s) != HELLO {
            continue;
        }
        let _ = s.write_all(format!("roomguard {}\n", std::process::id()).as_bytes());
        // Let the new copy hang up first, so the closing connection's leftovers
        // sit on its throwaway port and never on RoomGuard's.
        let _ = s.read(&mut [0u8; 1]);
        util::log("RoomGuard was opened again, showing this window instead");
        // On Windows the new copy raises this window itself (see below). The
        // same request through the window system covers macOS and Linux.
        if let Some(c) = ctx.lock().ok().and_then(|g| g.clone()) {
            c.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            c.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            c.send_viewport_cmd(egui::ViewportCommand::Focus);
            c.request_repaint();
        }
    }
}

/// Restores and raises the main window of the RoomGuard process `pid`. The
/// copy that was just started is allowed to bring a window to the front,
/// because you just opened it, so it does it rather than asking.
#[cfg(windows)]
fn raise_window_of(pid: u32) {
    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: extern "system" fn(isize, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn IsIconic(hwnd: isize) -> i32;
        fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        fn SetForegroundWindow(hwnd: isize) -> i32;
    }
    struct Search {
        pid: u32,
        found: isize,
    }
    extern "system" fn visit(hwnd: isize, lparam: isize) -> i32 {
        // SAFETY: lparam is the `Search` handed to EnumWindows below, which
        // stays alive and untouched elsewhere for the whole call.
        let search = unsafe { &mut *(lparam as *mut Search) };
        let mut owner = 0u32;
        // SAFETY: a plain Win32 query on a window handle EnumWindows gave us.
        let visible = unsafe {
            GetWindowThreadProcessId(hwnd, &mut owner);
            IsWindowVisible(hwnd) != 0
        };
        if owner == search.pid && visible {
            search.found = hwnd;
            return 0;
        }
        1
    }
    let mut search = Search { pid, found: 0 };
    // SAFETY: plain Win32 calls. The callback only uses `search` during EnumWindows.
    unsafe {
        EnumWindows(visit, &mut search as *mut Search as isize);
        if search.found != 0 {
            // 9 is SW_RESTORE (out of the taskbar), 5 is SW_SHOW.
            ShowWindow(search.found, if IsIconic(search.found) != 0 { 9 } else { 5 });
            SetForegroundWindow(search.found);
        }
    }
}

#[cfg(not(windows))]
fn raise_window_of(_pid: u32) {}
