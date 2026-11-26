# RoomGuard

RoomGuard turns your webcam into the sensors of a real security alarm system. It runs faithful, 1:1 replicas of three classic alarm panels and keypads:

- **RmG Classic (`PC1550` / `PC1500RK`)**: classic 6-zone LED keypad with side-hinged cover.
- **GuardSeries (`PC1832` / `PK5500`)**: 8-zone full-message 2x16 LCD keypad with bottom-hinged cover and 5 quick-function keys.
- **VistaGuard 20P (`VISTA-20P` / `VG6160V`)**: 8-zone alpha-display **voice** keypad with 1-inch dynamic speaker, spoken status and zone announcements, `[Code] + [Action]` commands, and exposed A-D quick keys.

Every panel follows its real installation manuals, programming guides, and hardware bus behavior down to the section and field numbers, LED masks, beeps, exit/entry urgency cadences, and factory defaults.

---

## Quick Start (Beginner Friendly)

You don't need any alarm experience to use RoomGuard:

1. **Launch RoomGuard.** The setup wizard walks you through picking a keypad, setting your 4-digit master code (default `1234`), choosing your webcam (or the built-in **Demo camera** if you don't have one), and optional phone alerts.
2. **Draw your zones on the Live view or Room map.** Mark your doorway (**Z01**), then use **Draw zone...** (or click **Draw** next to any zone in the **Zones** sidebar) to drag boxes for additional zones like windows, hallways, or extra doors directly on either the **Live** picture or **Room map**. Toggle **Contact** vs. **Motion** per zone right on its chip.
3. **Ignore your desk/chair (recommended).** Click **Ignore area** (or **Ignore desk**) and draw a box over where you sit at your computer. That way sitting at your desk won't block you from arming, while reaching an arm into the doorway or walking into the room still triggers immediately.
4. **Fresh empty-room calibration every launch (automatic & guided).** Every time RoomGuard starts, it runs a fresh empty-room calibration automatically (with an on-screen banner and 5-second countdown) so moved or new objects and today's lighting are learned accurately.
   - Step completely out of view and close any watched doors or windows during the 5-second countdown.
   - RoomGuard waits until **zero people** are visible, then scans ~36 frames of your empty room to learn today's exact background, per-pixel noise floor, and closed-contact images for every contact zone.
   - **Tip:** If you move furniture, open blinds, or bump the camera later in the session, click **Recalibrate** anytime to remap the room in seconds.

---

## What It Does

- **Three authentic panels.** Key sequences, LEDs, LCD displays, entry/exit delays with last-10-second urgency beeps, zone bypassing, troubles, alarm memory, user codes, schedules, macros, keypad blanking, 6-digit codes, and installer programming work just like the real hardware. See [the programming reference](docs/PROGRAMMING.md).
- **Live Panel Settings (no manual needed).** Open **Settings -> Panel** to adjust exit/entry delays, quick arm, buzzer/voice volume, chime, and zone definitions with plain-English checkboxes and sliders, synced live both ways with keypad programming.
- **Your webcam is a multi-zone sensor.** Zone 1 is your **Doorway** (entry/exit delay + visual door contact) and Zone 2 is **Room Motion** (interior motion detector, automatically inactive in Stay mode). You can also draw boxes for any additional panel zone (`Z01`-`Z08`) directly on the **Room map** or **Live** view, with per-zone **Contact** or **Motion** monitoring.
- **Smart person & limb detection.** Powered by pure-Rust neural networks (RTMDet-Nano + RTMPose-Tiny 17-joint skeleton, Kinect-style). Static objects like posters, hanging clothes, furniture, and lighting shifts are ignored, while a real person (or an arm/leg reaching into the doorway or a drawn zone) is caught even if their face isn't visible.
- **Fresh empty-room calibration every launch.** Automatically calibrates your empty room and closed contacts on startup with lighting-invariant edge and intensity matching, so new or moved objects and shadows don't cause false alarms while opening a watched door or window trips its zone immediately.
- **Offline face disarm.** Optionally enroll your face in **Settings -> Camera** so RoomGuard can automatically disarm during entry delay when it recognizes you. Everything stays 100% local on your computer.
- **Authentic synthesized audio.** Every keypad buzzer, VG6160V voice announcement, phone-line Contact ID tone, and all four sirens (**WAVE-2F Horn Siren**, **Master Blaster Motor Siren**, **RX-7C Indoor Piezo**, and **10 inch motor bell**) are synthesized live from physical models and measurements of the real hardware. Switching speakers or headphones moves audio seamlessly within ~1 second.
- **Simulated monitoring center & phone alerts.** A built-in operator chats and calls on alarms (simulated only, nobody is ever dispatched), and free push alerts with camera snapshots can be sent to your phone via [ntfy](https://ntfy.sh) with no account required.
- **Battery & CPU friendly.** Adaptive idle audio, background-priority body tracking, and automatic power-saving when minimized keep CPU, GPU, and laptop battery usage low.
- **Tamper-resistant.** Closing while armed requires your code, power loss or restarts come back armed automatically, and covering or unplugging the camera raises a real panel trouble.

---

## Keypad Cheat Sheet

Default Master Code on all three panels is **`1234`** (change it anytime in Setup or Settings).

### RmG Classic (`PC1550`) & GuardSeries (`PC1832` / `PK5500`)
- **Arm Away:** Enter your 4-digit code (or hold **Away** 2s on PC1832), then leave through the doorway during exit delay.
- **Arm Stay:** Enter your code and stay inside, or hold **Stay** 2s on PC1832 (bypasses Room Motion automatically).
- **Disarm / Silence:** Enter your 4-digit code (or 6-digit code if `[701]` option 5 is enabled on PC1832).
- **Bypass a zone:** Press `*` `1`, enter the zone number (`2` on PC1550, `02` on PC1832), then `#`.
- **Door Chime:** Press `*` `4` (or hold **Chime**) on PC1832, or `*` `6` + `[Master Code]` + `6` on PC1550.
- **Check Troubles:** Press `*` `2` (press `#` to silence trouble beeps).
- **Emergencies:** Hold **F** (Fire), **A** (Aux/Medical), or **P** (Panic) for 1 second on PC1550, or 2 seconds on PC1832.
- **Installer Programming:** `*` `8` + Installer Code (`1500` on PC1550, `5555` on PC1832).

### VistaGuard 20P (`VISTA-20P` / `VG6160V` Voice Keypad)
Uses `[Code] + [Action Key]` commands:
- **Arm Away:** `[Code] + 2` (or hold **AWAY** key D for 2s).
- **Arm Stay:** `[Code] + 3` (press `3` again right after for **NIGHT-STAY**).
- **Arm Maximum / Instant (no entry delay):** `[Code] + 4` (Max) or `[Code] + 7` (Instant).
- **Disarm / Silence:** `[Code] + 1` (OFF). After an alarm, enter `[Code] + 1` a **second time** to clear alarm memory (`Alarm Canceled`).
- **Bypass a zone:** `[Code] + 6` + 2-digit zone (`01`-`08`). Enter `[Code] + 6 + 00` or `[Code] + 1` while disarmed to clear all bypasses. (Enable `*23` in **Settings -> Panel** for quick bypass `[Code] + 6 + #`.)
- **Door Chime:** `[Code] + 9` while disarmed (speaks the opened zone name).
- **View Faults / Voice Status:** Press `*` to view or speak open zones, or `# + 0 + *` to hear system status (press `*` again within 10s to hear up to 3 faulted zones).
- **Clock, Event Log, Schedules & Macros:** `[Master] + # + 63` (Time/Date, `*` to edit), `# + 60` (100-event log), `# + 64` (Schedules), `# + 66` (Macros), `# + 7 / # + 8` (Outputs `01`-`18`).
- **Voice Volume & Message Center:** `# + 0 + 2` (`3` louder, `6` quieter, `4` voice on/off, `#` save), `# + 0 + 1` (record memo, `1` stop), `# + 0 + 3` (play memo).
- **Emergencies (hold 2s):** **FIRE** (Zone 95), **POLICE** (Zone 99), **MED** (Zone 96).
- **Installer Programming:** `4112 + 8 + 0 + 0`, then `*field` to edit, `#field` to review, `*99` to exit.

### Computer Keyboard Shortcuts
| Keys | Action |
| --- | --- |
| `0`-`9`, `*`, `#` | Keypad buttons |
| Hold `F`, `A`, `P` | Fire, Aux/Medical, Panic |
| `Enter` or `Esc` | `#` |
| Left / Right arrows | `<` / `>` (PC1832 LCD) |
| `F1`-`F5` | Stay, Away, Chime, Reset, Exit (PC1832) |

---

## Download and First Launch

Download the latest release from the [Releases page](../../releases):

- **Windows:** Run `roomguard-windows-x86_64.exe`. Because the binary isn't code-signed, SmartScreen may prompt; click **More info -> Run anyway** once.
- **macOS (Apple Silicon & Intel universal `.dmg`):** Open the `.dmg` and drag **RoomGuard** into Applications. On first launch, Control-click -> **Open** (or **System Settings -> Privacy & Security -> Open Anyway**) and allow Camera access.
- **Linux (`x86_64`):**
  ```sh
  chmod +x roomguard-linux-x86_64
  ./roomguard-linux-x86_64
  ```

---

## Phone Alerts in 2 Minutes (Free, No Account)

RoomGuard sends push alerts and snapshots via [ntfy](https://ntfy.sh):

1. Open **Settings -> Monitoring** and turn on **Send alerts to my phone**.
2. Install the free **ntfy** app on Android or iOS.
3. Tap **+** in ntfy and scan RoomGuard's QR code (or type the generated topic).
4. Click **Send a test alert** to verify delivery.

*(Your topic name acts as a secret key; click **New topic** anytime to rotate it, or point it to your own self-hosted ntfy server.)*

---

## Where Your Data Lives

Settings, salted/hashed codes, enrolled faces, room baselines, and history are stored separately from the executable so updates never wipe your data:

- **Windows:** `%APPDATA%\RoomGuard`
- **macOS:** `~/Library/Application Support/RoomGuard`
- **Linux:** `~/.local/share/RoomGuard`

---

## Building from Source

Install Rust from [rustup.rs](https://rustup.rs), then:

```sh
cargo test --release
cargo run --release
```

On Linux, install the system libraries first:

```sh
sudo apt-get install pkg-config clang libasound2-dev libudev-dev libv4l-dev \
  libxkbcommon-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libgl1-mesa-dev
```

---

## Honest Limits

- One webcam cannot measure true 3D depth; for best accuracy, mount and aim the camera steadily at the doorway and room, and click **Recalibrate** whenever the camera moves.
- Body tracking requires normal room lighting; pitch-dark rooms cannot be seen without an IR camera.
- RoomGuard is an enthusiast/hobby room alarm simulator, not a UL-listed life-safety or certified security system. Do not rely on it to protect life or property.

---

## License

Copyright 2026 Kaushal Bhingaradia. Licensed under the [Apache License 2.0](LICENSE). See [NOTICE](NOTICE) for third-party credits.

RoomGuard emulates classic `PC1550`, `PC1832`, and `VISTA-20P` panel behavior under its **RmG Classic**, **GuardSeries**, and **VistaGuard 20P** names. It is not made by, endorsed by, or affiliated with any hardware manufacturer. All keypad graphics are drawn in code.
