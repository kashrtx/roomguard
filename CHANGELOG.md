# Changelog

Every release lists what changed. The release workflow copies the section for
each version onto its GitHub release page.

## [2.2.0] - 2026-11-26

RoomGuard 2.2.0 adds intuitive multi-zone drawing directly on both the **Room map** and **Live** camera views, runs a fresh empty-room calibration on every launch with clear on-screen guidance, embeds native app icons on Windows, macOS, and Linux builds, and fixes false zone-bypass indicators and unexpected instant alarms.

### Highlights & New Features

- **Intuitive Multi-Zone Drawing (Map & Live Views):**
  - Draw, redraw, or remove any active panel zone (`Z01`–`Z06` on PC1550; `Z01`–`Z08` on PC1832 and VistaGuard 20P) directly on either the **Room map** or **Live** camera view using the color-coded zone picker, one-click **Draw** / **Redraw** buttons in the **Zones** sidebar, or by assigning learned map portals to any zone.
  - Each drawn zone gets a distinct color pill on the video/map, an interactive chip with one-click redraw and remove (`×`), and a per-zone **Contact / Motion** toggle so doors and windows can watch open/closed visual state while interior areas watch people and motion.
  - Pressing `Escape` cancels an active drag-to-draw cleanly without sending `#` to the keypad.
- **Fresh Calibration Every Launch:**
  - RoomGuard no longer reuses stale empty-room baselines from previous sessions — every launch (and every webcam switch) begins a fresh calibration automatically once streaming while disarmed, locking today's room layout, new or moved objects, lighting, and closed contacts for every contact-enabled zone.
  - Clear, prominent calibration banners and video overlays explain why calibration is needed and guide you through stepping out of view, with quick **Restart 5s**, **Skip for now**, and **Recalibrate** controls.
- **Native App Icons on Every Platform:**
  - **Windows:** Rebuilt `assets/icon/icon.ico` with standard 32-bit BGRA DIB frames (16–128 px) plus 256 px PNG, and replaced external `rc.exe`/`windres.exe` calls with a zero-dependency pure-Rust COFF `.rsrc` resource embedder in `build.rs` so the executable icon and `VS_VERSIONINFO` metadata embed reliably on every toolchain and path.
  - **macOS:** Rebuilt `assets/icon/icon.icns` with all standard sizes (`icp4`–`ic14`), updated `Info.plist`, and fixed the release `.dmg` staging so `RoomGuard.app` (with volume icon and `/Applications` shortcut) mounts cleanly.
  - **Linux:** Added desktop launcher & `256×256` hicolor icon installation, `StartupWMClass=roomguard` window matching, and `gio` custom-icon metadata support.

### Bug Fixes

- **Fixed false "Zone Bypassed" display in Stay / Auto-Stay mode:** Automatic Stay-mode interior bypasses are now distinguished from manual user bypasses — VistaGuard 20P displays `** ALL SECURE **` instead of falsely flashing `ZONE BYPASSED`, and the Zones sidebar clearly shows `Stay off` instead of `Bypassed` unless you explicitly bypassed a zone.
- **Fixed unexpected instant alarms:**
  - Fixed a lighting-change bug in the vision limb check that could treat the entire room as active motion when brightness shifted, and tightened limb checks to require confirmed (`Normal`+) detections while excluding people still inside the doorway or seated in an Ignore desk box from prematurely tripping Zone 2.
  - Added Delay-first zone settling with an anti-race hold in the engine so walking through a doorway never lets an Interior/Follower zone beat the Delay zone by a fraction of a second and fire a false instant alarm.
  - Separated brief camera `fault` drops (trouble only) from true `tamper` alarms and fixed VistaGuard 20P exit-error handling so an open interior zone at exit expiry starts Entry Delay instead of an instant zero-delay alarm.
  - Shortened camera zone release hold from `2.5s` to `1.0s` so exiting cleanly clears zones before exit delay ends.

## [2.1.0] - 2026-11-25

RoomGuard 2.1.0 introduces the **VistaGuard 20P** voice keypad, guided empty-room calibration with lighting-invariant closed-door detection, limb-aware person tracking with an **Ignore desk** workflow, studio-matched sirens and keypad acoustics, zero-delay keystroke audio, and line-by-line manual accuracy across all three panels.

### Highlights & New Features

- **VistaGuard 20P (`VISTA-20P` / `VG6160V`) Voice Keypad:**
  - Authentic `[Code] + [Action Key]` firmware: `1` OFF (two-step alarm memory clear + `*86` `Alarm Canceled`), `2` AWAY (with `*84` Auto-Stay), `3` STAY (and `3` again for NIGHT-STAY with `*81`), `4` MAXIMUM (Away with zero entry delay, Auto-Stay to INSTANT), `5` TEST (`0` walk test, `1` dialer test), `6` BYPASS (`01`–`08` or `#` quick bypass `*23`), `7` INSTANT, `8` CODE (user management), `9` CHIME (with spoken zone names), `*` fault viewer / exit restart (`*91`), `# + 0 + 2` voice volume & toggle, `# + 0 + *` voice status, and `4112 + 8 + 0 + 0` installer programming.
  - Real 6160V voice ROM (`Disarmed`, `Ready to arm`, `Not ready to arm`, `Exit now` preserved from original recordings, plus natural adult female zone/status vocabulary matched to the 1-inch plastic speaker frequency curve).
  - Measured 6160V speaker acoustics: 1360 Hz / 1019 Hz continuous burglary warble, 46 ms key chirp, 1019 Hz confirmation tone, 10-second any-key keypad silence during alarm, and live ready-state evaluation right when announcing status after disarm.
- **Guided Empty-Room Calibration & Door Contact:**
  - Click **Calibrate empty room** (or **Recalibrate empty room**) on the Camera card: a 5-second countdown lets you close the door and step out of view, pauses automatically if anyone is still visible, and scans 36 frames to save your room's exact background, per-pixel noise floor, and closed-door image permanently to disk.
  - `DoorWatch` matches the calibrated `160×mh` baseline 1:1 with lighting-invariant signed gradients + intensity correlation + mean-adjusted pixel change, eliminating false "Open" readings on shut doors.
- **Smart Person & Limb Detection + Ignore Desk Box:**
  - Powered by pure-Rust RTMDet-Nano + RTMPose-Tiny 17-joint body tracking: static objects, clothes, posters, and brightness shifts are ignored, while full bodies and partial limbs (arms/wrists/legs) reaching into the doorway or room trigger immediately.
  - Putting an **Ignore desk** box over your chair/desk lets you arm effortlessly while seated without masking arms reaching into the doorway or walking out into the room.
- **Live Settings -> Panel Tab:**
  - Plain-English controls for delays, bell cut-off, door chime, quick arm, buzzer/voice volume, auto-arm schedule, and zone definitions synced live both ways with keypad programming.

### Audio Latency, Sync & Sirens

- **Zero-delay keystroke audio:** Moved Windows/OS default-device switching onto a dedicated low-priority watcher thread so device enumeration never stalls a keypress beep. Switching speakers or headphones moves live audio seamlessly within ~1 second.
- **Zero overlap & pop-free speech:** Keypresses smoothly fade out in-progress 6160V speech (4 ms ramp), cancel stale pending status announcements, and cut cleanly through steady entry, last-10-second pulsing entry, and Hi-Lo burglary tones.
- **Rebuilt sirens & keypads from real hardware measurements:**
  - **WAVE-2F Horn Siren:** 2082 Hz steady; 8.4 Hz RC-rise warble (1570–2230 Hz).
  - **Master Blaster Motor Siren:** 1833 Hz rotor with realistic spin-up, coast-down, and 2nd/3rd harmonics.
  - **RX-7C Indoor Piezo:** 3555 Hz (±230 Hz) warble at 15.4 Hz with oscillator start swoop.
  - **PK5500 (`PC1832`) & PC1500RK (`PC1550`):** 2295 Hz PK5500 piezo tone, 6-beep code arming, 1 Hz -> 3-beep-burst exit urgency, and `beep-beep-beeeep` last-10-second entry delay urgency on both panels.

### Manual Accuracy, Performance & Reliability

- **1:1 manual fidelity across all three panels:** Verified factory defaults, zone attribute tables, loop response times (500 ms / 400 ms / 350 ms), silent double-length PC1832 Stay exit delay, `[015]` opt 2 audible Panic, `[018]` opt 5 buzzer-follows-bell, PC1550 `[32]` opt 1 AC trouble option & `[34]` opt 2/4 exit squawk and entry urgency, PC1832 `[*][6]` date validation, PK5500 LCD `[kp076]` options and `0–9` brightness/contrast levels, and Vista contextual hints.
- **Smooth & freeze-free:** Background atomic saving, dedicated battery polling thread, non-blocking 3-second cap on offline armed-shutdown alerts, virtualized event history & chat rendering, GPU rendering with OpenGL fallback, panic recovery on engine/camera/UI threads, and single-instance window focus.
- **Cross-platform builds:** Native builds for Windows (`x86_64`), macOS universal (`Apple Silicon + Intel .dmg` with explicit HFS+ staging size), and Linux (`x86_64`).

## [2.0.0] - 2026-11-24

A complete rewrite as a native desktop app in Rust for Windows, macOS and Linux.

### Panels
- Two panels emulated from their manuals: the DSC PC1550 with an LED keypad and the DSC PowerSeries PC1832 with a 2 x 16 LCD keypad.
- Real key sequences and menus: bypass, trouble view, alarm memory, user codes and attributes, user functions, quick arm, quick exit, night arming, door chime, sensor reset, command outputs and the event buffer.
- Installer programming with the real section numbers, including toggle options, data sections, hex entry and factory reset.
- Entry and exit delays, interior follower zones, stay and away arming, swinger shutdown, keypad lockout, auto arm, duress codes and fire, auxiliary and panic keys.
- Troubles from the computer itself: unplugging is an AC failure, a low laptop battery is a low battery, missing speakers are a bell trouble, and a covered or unplugged camera is a zone tamper or fault.
- The panel comes back armed after the app is closed, crashes or loses power, like a real panel.

### Camera
- The webcam is the room's sensors: the doorway is zone 1 and the rest of the room is zone 2.
- Multi person tracking with a Kalman filter that predicts where each person is heading.
- A learned room map: doorways from foot traffic, walking paths, and spots that move on their own.
- Door watching: an open door trips the doorway zone even with nobody in view.
- Pan and tilt support for USB PTZ webcams and OBSBOT cameras, with lead aiming, search plans when someone gets away, and a room mapping sweep that calibrates the motor and stitches a panorama.
- Offline face recognition that can disarm during the entry delay, with a check that blocks a photo held still.
- A demo camera for trying everything without a webcam.

### Sound
- Keypad buzzers and four sirens synthesized from physical models: Ademco WAVE-2F, Klaxon Master Blaster, ADT RX-7C and a motor bell.
- Optional phone line sounds of the communicator dialing out with Contact ID or pulse format.

### Monitoring
- A simulated monitoring center that messages and calls you, asks for your passcode, and dispatches or cancels. Dispatching is simulated.
- Phone alerts with camera snapshots through ntfy, retried eight times before a communication trouble shows.

### App
- A first run setup that covers your name, keypad, master code, camera, phone alerts and sign in.
- Closing while armed asks for your code. Wrong codes count toward the keypad lockout.
- Optional opening at sign in, and keeping the computer awake while armed.
- Everything is saved to a data folder that updates and reinstalls never touch, with crash safe writes and a backup copy.
- An update checker that shows the release notes and asks before installing.
- Licensed under the Apache License 2.0.

## Earlier browser versions

### 1.1.1

- **Fixed: face teaching froze on the first pose.** A leftover beep call from the sound rewrite crashed it on every capture, silently, so it never got past the first view.
- **Only one bridge can run.** On Windows a second copy could quietly share the port, so saving failed at random and an old copy could serve an old app. Starting the bridge again now just opens the app, and an older bridge still running is reported.
- **Face teaching is easier.** It waits for the room map instead of fighting the sweep, works from further away, zooms in on you with a moving camera, and says why if it can't see a face.
- **Finds small, far-away faces.** When it sees a person's body but no face, it looks closely at their head at full resolution.
- **Saving survives a bridge restart.** The app fetches the bridge's new key and retries.

### 1.1.0

- **Whole-body detection.** People are tracked by body as well as face. It tilts up from a tall person's body to find their face, keeps knowing who someone is while they're turned around, and a person whose face it never sees still trips the alarm (zone 02) and gets a full-body photo.
- **Room mapping replaces the old calibration.** The first start sweeps across the room in steps, calibrating from many measurements and stitching a panorama. Later starts do a quick check, and re-map automatically if the camera turns differently.
- **Room map under the video.** Shows the view, entry way, people, last sighting and heading. Click to look somewhere; drag on it to mark the entry.
- **Smarter searching.** Leads fast runners further, investigates movement while searching, and sweeps the whole room from the side someone ran toward before going home. "Keep searching for" sets how long.
- **Fixed:** a crash when following someone by face whose face briefly left the picture; posters' bodies counting as alive when the frame edge cut them off.

### 1.0.0

- First release: face recognition with liveness checks, two original alarm keypads with realistic piezo sounds, zones, user and duress codes, programming, entry way and ignore areas, gimbal following with direction-of-travel search, recordings, and automatic saving through the bridge.
