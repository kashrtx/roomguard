//! "Open RoomGuard when I sign in", using each system's ordinary per user
//! mechanism: the Run key under the current user on Windows, a LaunchAgent
//! on macOS and an autostart entry on Linux. It's off unless you turn it on,
//! needs no administrator rights, and turning it off removes it completely.

use std::path::Path;

pub fn set(enabled: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    apply(enabled, &exe)
}

#[cfg(windows)]
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

#[cfg(windows)]
fn apply(enabled: bool, exe: &Path) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu.create_subkey(RUN_KEY).map_err(|e| e.to_string())?;
    if enabled {
        let value = format!("\"{}\" --minimized", exe.display());
        key.set_value("RoomGuard", &value).map_err(|e| e.to_string())
    } else {
        match key.delete_value("RoomGuard") {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(windows)]
pub fn is_enabled() -> bool {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>("RoomGuard"))
        .is_ok()
}

#[cfg(target_os = "macos")]
fn plist_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join("Library/LaunchAgents/com.kaushalbhingaradia.roomguard.plist"))
}

#[cfg(target_os = "macos")]
fn apply(enabled: bool, exe: &Path) -> Result<(), String> {
    let path = plist_path().ok_or("no home folder")?;
    if !enabled {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let exe = exe.display().to_string().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.kaushalbhingaradia.roomguard</string>
  <key>ProgramArguments</key><array><string>{}</string><string>--minimized</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#,
        exe
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, plist).map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
pub fn is_enabled() -> bool {
    plist_path().map_or(false, |p| p.exists())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|c| c.join("autostart/roomguard.desktop"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn apply(enabled: bool, exe: &Path) -> Result<(), String> {
    let path = desktop_path().ok_or("no config folder")?;
    if !enabled {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=RoomGuard\nComment=Camera based room alarm\nExec=\"{}\" --minimized\nIcon=roomguard\nStartupWMClass=roomguard\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, entry).map_err(|e| e.to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
pub fn is_enabled() -> bool {
    desktop_path().map_or(false, |p| p.exists())
}
