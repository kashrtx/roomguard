# RoomGuard

RoomGuard uses your webcam to monitor room entry and motion, with software emulations of three classic alarm panels and keypads.

- **RmG Classic (`PC1550` / `PC1500RK`)**: classic 6-zone LED keypad with side-hinged cover.
- **GuardSeries (`PC1832` / `PK5500`)**: 32-zone panel with a full-message 2×16 LCD keypad with bottom-hinged cover and 5 quick-function keys.
- **VistaGuard 20P (`VISTA-20P` / `VG6160V`)**: 48-zone panel with an alpha-display **voice** keypad with 1-inch dynamic speaker, spoken status and zone announcements, `[Code] + [Action]` commands, and exposed A-D quick keys.

The keypads follow the manuals for supported commands, displays, tones, and programming. The programming reference describes the available controls and current hardware differences.

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

- **Three classic panels.** Key sequences, LEDs, LCD displays, entry/exit delays with last-10-second urgency beeps, zone bypassing, troubles, alarm memory, user codes, schedules, macros, keypad blanking, 6-digit codes, and installer programming are available according to the selected model. See [the programming reference](docs/PROGRAMMING.md).
- **Live Panel Settings (no manual needed).** Open **Settings -> Panel** to adjust exit/entry delays, quick arm, buzzer/voice volume, chime, and zone definitions with plain-English checkboxes and sliders, synced live both ways with keypad programming.
- **Your webcam is a multi-zone sensor.** Zone 1 is your **Doorway** (entry/exit delay + visual door contact) and Zone 2 is **Room Motion** (interior motion detector, automatically inactive in Stay mode). You can also draw boxes for any additional panel zone (`Z01`-`Z08`) directly on the **Room map** or **Live** view, with per-zone **Contact** or **Motion** monitoring.
- **Smart person & limb detection.** Powered by pure-Rust neural networks (RTMDet-Nano + RTMPose-Tiny 17-joint skeleton, Kinect-style). Static objects like posters, hanging clothes, furniture, and lighting shifts are ignored, while a real person (or an arm/leg reaching into the doorway or a drawn zone) is caught even if their face isn't visible.
- **Fresh empty-room calibration every launch.** Automatically calibrates your empty room and closed contacts on startup with lighting-invariant edge and intensity matching, so new or moved objects and shadows don't cause false alarms while opening a watched door or window trips its zone immediately.
- **Offline face disarm.** Optionally enroll your face in **Settings -> Camera** so RoomGuard can automatically disarm during entry delay when it recognizes you. Everything stays 100% local on your computer.
- **Authentic synthesized audio.** Every keypad buzzer, VG6160V voice announcement, phone-line Contact ID tone, and all four sirens (**WAVE-2F Horn Siren**, **Master Blaster Motor Siren**, **RX-7C Indoor Piezo**, and **10 inch motor bell**) are synthesized live from physical models and measurements of the real hardware. Under **Settings → Sound**, choose separate outputs for keypad tones and voice, and for the siren. Leave the siren on **Same as keypad** to use one speaker. The Test buttons check each selected output.
- **Simulated monitoring center & phone alerts.** A built-in operator chats and calls on alarms (simulated only, nobody is ever dispatched), and free push alerts with camera snapshots can be sent to your phone via [ntfy](https://ntfy.sh) with no account required.
- **Battery & CPU friendly.** Adaptive idle audio, background-priority body tracking, and automatic power-saving when minimized keep CPU, GPU, and laptop battery usage low.
- **Tamper-resistant.** Closing while armed requires your code, the panel can restore its armed state on restart, and covering or unplugging the camera raises a real panel trouble.

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
- **Bypass a zone:** `[Code] + 6` + 2-digit zone (`01`–`48`). Enter `[Code] + 6 + 00` or `[Code] + 1` while disarmed to clear all bypasses. (Enable `*23` in **Settings -> Panel** for quick bypass `[Code] + 6 + #`.)
- **Door Chime:** `[Code] + 9` while disarmed (speaks the opened zone name).
- **View Faults / Voice Status:** Press `*` to view or speak open zones, or `# + 0 + *` to hear system status (press `*` again within 10s to hear up to 3 faulted zones).
- **Clock, Event Log, Schedules & Macros:** `[Master] + # + 63` (Time/Date, `*` to edit), `# + 60` (100-event log), `# + 64` (Schedules), `# + 66` (Macros), `# + 7 / # + 8` (Outputs `01`-`18`).
- **Voice Volume & Message Center:** `# + 0 + 2` (`3` louder, `6` quieter, `4` voice on/off, `#` save), `# + 0 + 1` (record an in-memory memo, `1` stops), `# + 0 + 3` (play or replay the memo).
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

## Test speakers and microphone

Open **Settings → Sound** and choose a device for **Keypad & voice**, **Siren**, and **Microphone**. Use **Test keypad** and **Test siren** to check the selected outputs. On Windows, use the same output names shown in Sound settings. A speaker and a headphone jack can share a single Windows output, depending on the sound driver.

Select **Test microphone**, speak for three seconds, and listen for playback through **Keypad & voice**. The same microphone records VISTA memos. If Windows blocks recording, allow desktop app microphone access under **Settings → Privacy & security → Microphone**.

## Use RoomGuard as a touchscreen keypad

Open **Layout** in the top bar to show or hide **Camera & zones** and **Monitoring & history** independently. Choose **Keypad only** to give the keypad the whole wall. **Exit keypad view** restores your sidebar choices.

Turn on **Lock keypad view when armed** to switch to the keypad automatically when arming begins. The sidebars and setup controls stay hidden until you disarm by touch or keyboard. Monitoring and notifications keep running in keypad view.

## Phone alerts and quiet monitoring

1. Open **Settings → Phone alerts** and turn on **Send alerts to my phone**.
2. Install ntfy on your phone and subscribe with the QR code. Tap **Send a test alert** and check your phone.
3. Choose **Quiet intrusion alerts** to keep burglary and tamper alarms quiet. Keypad tones and emergency alarms still follow the panel. Choose **Use siren** for the panel's programmed alarm sounds.
4. Turn on **Include camera evidence when available** to attach a snapshot. Enable **Send a short motion GIF for intrusion events** to include about three seconds around a burglary, entry, or unknown-person event. If an entry countdown ends in a burglary alarm, RoomGuard can attach the clip from when the door first opened. Open the attachment on your phone to watch it. RoomGuard falls back to a JPEG when it cannot make a GIF from the frames it has, or to text when the camera is unavailable.

Phone alerts report actual RoomGuard events. The simulated operator is a separate practice feature. Keep a public ntfy topic private, or use access control on your ntfy server.

The display can be off while RoomGuard runs. Keep the computer awake, the camera powered, and RoomGuard focused for keypad input from a paired wireless keyboard. Type your code to disarm a DSC panel. On Vista, enter your code followed by **1 (OFF)**.

For a dedicated external siren speaker, use a stable, ventilated spot away from vents, fabrics, and exit paths. Try **Settings → Sound → Test** after the speaker has been idle and check where it can be heard.

## Home Assistant sensors

Use [the Windows Home Assistant lab](docs/HOME_ASSISTANT_WINDOWS.md) to test with virtual sensors and an alarm panel on your Windows PC.

1. In your Home Assistant profile, create a long-lived access token. In RoomGuard open **Settings → Home Assistant → Connection**, enter the base URL and token, and select **Connect**. Use HTTPS when connecting remotely. RoomGuard saves the token in its local configuration.
2. Search for an entity and choose an enabled zone. For a door or motion sensor, **Is on** usually means the zone is open. For a numeric sensor, choose **Above** or **Below** and enter a threshold. **Changes** briefly opens a zone on a live state change, useful for event entities. Zone definitions still decide which alarm behavior applies.
3. Open and close the sensor while disarmed and check its zone on the keypad. A disconnected or unavailable sensor appears as a zone fault. On connection and reconnection, RoomGuard first reads the current states and then reacts to live changes.

Any readable entity with a compatible state can be a zone source. Each zone has one source and the panel still sets the number of zones. Pairing, battery status, and device configuration stay in Home Assistant. Select a camera entity and choose **Load snapshot** for one on-demand JPEG preview. For camera motion, assign a motion entity exposed by Home Assistant. Previewing a camera starts no continuous stream or tracking worker.

Turn off **Use webcam** in **Settings → Home Assistant** or **Settings → Camera** to stop webcam capture and tracking. Home Assistant zones remain active and phone alerts can still send text.

### Follow a physical alarm panel

Connect the real panel to Home Assistant through a compatible bridge, then select its partition in **Settings → Home Assistant → Follow physical panel**. This works with the Classic LED, PowerSeries LCD, and VISTA voice layouts. Arm and disarm at the physical keypad. RoomGuard shows the states the bridge provides, including entry delay and alarm, and applies Stay or Away to its own zones. Night uses the VISTA Night-Stay zone list on the VISTA layout and Stay on the two DSC layouts. Vacation and custom bypass use RoomGuard’s Away rules.

Phone alerts can include evidence from RoomGuard’s camera for a physical panel alarm. The panel link leaves the physical siren in charge of those alarms. RoomGuard still monitors its own zones and uses its local siren and quiet-alert settings for its own alarms. A lost connection shows a fault and keeps the last confirmed arming mode until the link returns.

[Envisalink](https://www.home-assistant.io/integrations/envisalink/), the [DSC Keybus Interface](https://github.com/taligentx/dscKeybusInterface), and [ESPHome VISTA ECP](https://github.com/Dilbert66/esphome-vistaECP) are examples of bridges. RoomGuard needs the integration to publish an `alarm_control_panel` entity for the partition you want to follow.

## Experimental builds

The experimental branch publishes prereleases via GitHub Actions. Download the asset marked **experimental** for your platform. Experimental builds use their own data directory and check only experimental prereleases under **Settings → Updates**. Stable builds check stable releases.

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
