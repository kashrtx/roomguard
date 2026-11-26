//! Desktop integration for Linux: installs RoomGuard's embedded app icon and
//! `.desktop` launcher entry into the user's XDG data directories on first run
//! (or when the binary moves), and attaches a custom file icon via `gio` when
//! available so file managers and docks show RoomGuard's icon automatically.

#[cfg(all(unix, not(target_os = "macos")))]
pub fn ensure_linux_icon() {
    static ICON_256: &[u8] = include_bytes!("../assets/icon/icon-256.png");
    let Some(data_dir) = dirs::data_dir() else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let icon_dir = data_dir.join("icons/hicolor/256x256/apps");
    let icon_path = icon_dir.join("roomguard.png");
    let _ = std::fs::create_dir_all(&icon_dir);
    let need_icon = std::fs::metadata(&icon_path).map(|m| m.len() as usize != ICON_256.len()).unwrap_or(true);
    if need_icon {
        let _ = std::fs::write(&icon_path, ICON_256);
    }
    let apps_dir = data_dir.join("applications");
    let _ = std::fs::create_dir_all(&apps_dir);
    let desktop_path = apps_dir.join("roomguard.desktop");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=RoomGuard\nComment=Camera based room alarm\nExec=\"{}\"\nIcon={}\nStartupWMClass=roomguard\nTerminal=false\nCategories=Utility;Security;\n",
        exe.display(),
        icon_path.display()
    );
    let cur = std::fs::read_to_string(&desktop_path).unwrap_or_default();
    if cur != entry {
        let _ = std::fs::write(&desktop_path, entry);
    }
    // Best-effort custom file icon for GNOME Files / Nautilus / Thunar / Nemo.
    let icon_uri = format!("file://{}", icon_path.display());
    let _ = std::process::Command::new("gio")
        .args(["set", "-t", "string"])
        .arg(&exe)
        .args(["metadata::custom-icon", &icon_uri])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn ensure_linux_icon() {}
