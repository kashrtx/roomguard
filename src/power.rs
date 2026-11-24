//! The computer's power state. Unplugging a laptop is the panel's AC
//! failure, and a low battery while unplugged is its low battery trouble.

use crate::util;

#[derive(Clone, Copy, Debug)]
pub struct PowerState {
    pub on_ac: bool,
    pub has_battery: bool,
    pub percent: f32,
}

pub struct Power {
    manager: Option<starship_battery::Manager>,
}

impl Power {
    pub fn new() -> Power {
        let manager = match starship_battery::Manager::new() {
            Ok(m) => Some(m),
            Err(e) => {
                util::log(&format!("battery status unavailable: {}", e));
                None
            }
        };
        Power { manager }
    }

    /// A desktop without a battery always reads as on AC power.
    pub fn read(&mut self) -> PowerState {
        let desktop = PowerState { on_ac: true, has_battery: false, percent: 1.0 };
        let Some(m) = self.manager.as_ref() else {
            return desktop;
        };
        let batteries = match m.batteries() {
            Ok(b) => b,
            Err(_) => return desktop,
        };
        let mut any = false;
        let mut discharging = false;
        let mut sum = 0.0f32;
        let mut count = 0.0f32;
        for b in batteries.flatten() {
            any = true;
            sum += b.state_of_charge().get::<starship_battery::units::ratio::ratio>();
            count += 1.0;
            if b.state() == starship_battery::State::Discharging {
                discharging = true;
            }
        }
        if !any {
            return desktop;
        }
        PowerState { on_ac: !discharging, has_battery: true, percent: if count > 0.0 { sum / count } else { 1.0 } }
    }
}

/// Keeps the computer from going to sleep while the system is armed, using
/// each operating system's normal, documented way of doing it. Nothing
/// lingers if RoomGuard quits or crashes: the helper processes on macOS and
/// Linux end as soon as RoomGuard's process ends.
pub struct KeepAwake {
    on: bool,
    #[allow(dead_code)]
    child: Option<std::process::Child>,
}

impl KeepAwake {
    pub fn new() -> KeepAwake {
        KeepAwake { on: false, child: None }
    }

    /// Call from one long lived thread; Windows tracks this per thread.
    pub fn set(&mut self, want: bool) {
        if want == self.on {
            return;
        }
        self.on = want;
        #[cfg(windows)]
        {
            #[link(name = "kernel32")]
            extern "system" {
                fn SetThreadExecutionState(flags: u32) -> u32;
            }
            const ES_CONTINUOUS: u32 = 0x8000_0000;
            const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;
            let flags = if want { ES_CONTINUOUS | ES_SYSTEM_REQUIRED } else { ES_CONTINUOUS };
            // SAFETY: a plain Win32 call with no pointers.
            unsafe {
                SetThreadExecutionState(flags);
            }
        }
        #[cfg(not(windows))]
        {
            if want {
                let pid = std::process::id().to_string();
                let spawned = if cfg!(target_os = "macos") {
                    std::process::Command::new("caffeinate").args(["-i", "-w", pid.as_str()]).spawn()
                } else {
                    std::process::Command::new("systemd-inhibit")
                        .args(["--what=sleep:idle", "--who=RoomGuard", "--why=RoomGuard is armed", "--mode=block", "tail", "-f", "/dev/null", "--pid", pid.as_str()])
                        .spawn()
                };
                match spawned {
                    Ok(c) => self.child = Some(c),
                    Err(e) => util::log(&format!("couldn't keep the computer awake: {}", e)),
                }
            } else if let Some(mut c) = self.child.take() {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        util::log(if want { "keeping the computer awake while armed" } else { "the computer may sleep again" });
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        self.set(false);
    }
}
