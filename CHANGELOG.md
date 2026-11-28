# Changelog

Every release lists what changed. The release workflow copies the section for
each version onto its GitHub release page.

## [2.2.0] - 2026-11-26

RoomGuard 2.2.0 brings wire-level panel accuracy across all three emulated keypads, a rebuilt VG6160V voice bank with real keypad recordings and uniform loudness, full VistaGuard 20P advanced keypad functions (clock, event log, schedules, macros, outputs, and message center), multi-zone drawing on both the Room map and Live camera views, automatic fresh empty-room calibration on every launch, and native platform app icons.

### Voice Keypad & VistaGuard 20P Upgrades

- **Rebuilt VG6160V Voice Bank & Loudness Equalization:**
  - Keeps 8 real keypad voice words (`disarmed`, `not ready to arm`, `ready to arm`, `exit now`, `armed`, `stay`, `away`, `fire`) alongside pitch-matched, band-limited generated vocabulary so mixed phrases sound like the same speaker.
  - Perceptual loudness equalization balances all 76 voice ROM words within 0.6 dB so every word ("Armed", "Instant", "Stay", "Away", zone names) plays at uniform volume.
  - Added the authentic low 1019 Hz `tu-tu-tu` arming confirmation cadence for Stay, Night-Stay, and Instant modes, full arming start and completion voice announcements (including `exit now` only when leaving), and keypress sounder ducking so typing a code never overlaps with countdown beeps.
  - Authentic bypass voice behavior: quiet confirmation beep only during bypass entry, `"disarmed, bypassed"` / `"armed <mode>, zone bypassed"` on status and arming (only for manual bypasses, never Stay auto-bypass), and silent voice chime on bypassed zones.
- **Full VistaGuard 20P Keypad Features & Bus Accuracy:**
  - Added `[Master] + # + 63` Real-Time Clock view and edit with blinking HD44780 underline cursor, plus `*57` function key option `02` (one-touch Time/Date display), `07` (Output toggle), and `09`-`12` (Macro keys).
  - Added `[Master] + # + 60` 100-Event Contact ID History Log viewer (`*` older, `#` newer, `*90` event log filter), `# + 64` live schedules, `# + 66` 4-slot macro programming, `# + 7` / `# + 8` output device control (`01`-`18`), Voice Message Center (`# + 0 + 1` record, `1` stop, `# + 0 + 3` play with steady/flashing `MSG` LED), user attributes and authority levels 0-4 (Arm-Only, Guest, Duress CID 121, Partition Master), dialer test (`PHONE OKAY` / `COMM. FAILURE`), `[Code] + 6 + 00` bypass clear, `# + 3 + 3` quick Night-Stay, `*197` exit countdown interval, `*198` partition display, GoTo Partition, `*26` Chime by Zone, and double-`*` fault announcements.
  - Multi-condition LCD cycling rotates latched Alarm Memory with Disarmed every 2.5 s until the second `[Code] + 1`, cycles multiple simultaneous troubles (`AC LOSS`, `SYSTEM LO BAT`, `BELL FAILURE`, `COMM. FAILURE`, `CHECK XX`), and reports Exit Error (`0xEA` / CID 374) and authentic closing Contact ID codes (`401`, `403`, `408`, `441`, `459`).

### RmG Classic (PC1550) & GuardSeries (PC1832 / PK5500) Wire-Level Accuracy

- **GuardSeries PC1832 / PK5500:**
  - Implemented exact wire status screens across normal, arming, and menu states: `System Ready / Force Arm` (`0x02`), distinct `Armed Stay / No Entry Delay` (`0x06`) and `Armed Away / No Entry Delay` (`0x16`), `Arming / No Entry Delay` (`0x09`), continuous `Quick Exit in / Progress` (`0x0B`), `Entry Delay / After Alarm` (`0x0D`) with immediate pulsing urgency tone, `Invalid / Access Code` (`0x8F`), `Function Not / Available` (`0x0E`/`0x8E`), `Command O/P` active display (`0x33`), and confirmation banners for Door Chime, Auto-Arm, and Stay/Away reactivation.
  - Armed `*` menu (`0xB8`) filters to the 6 armed-valid functions only.
  - Implemented Keypad Blanking (`[016]` options 3 & 4: blanks LCD, backlight, and LEDs after 30 s idle, flashing Trouble when present, with optional access-code wake) and 6-digit access codes (`[701]` option 5, seamlessly matching legacy 4-digit codes with trailing `00`).
  - Idle scroll now includes `Bypass Active / (*1) to View`, armed scroll honors `[016]` option 7, and armed Trouble LED honors `[013]` option 3.
- **RmG Classic PC1550 / PC1500RK:**
  - Immediate prompt LED feedback on `*5`/`*6`/`*8` (Memory + Bypass + Trouble flashing), `*1` (Bypass flashing), and `*9` (Armed flashing), armed Memory/Fire trouble LED accuracy, clock trouble clearing on `*6`, and measured 100 ms key beep timing.

### Multi-Zone Drawing, Calibration & Platform Polish

- **Intuitive Multi-Zone Drawing (Map & Live Views):**
  - Draw, redraw, or remove any active panel zone (`Z01`-`Z06` on PC1550; `Z01`-`Z08` on PC1832 and VistaGuard 20P) directly on either the **Room map** or **Live** camera view with per-zone **Contact / Motion** monitoring and clean `Escape` cancel.
- **Fresh Calibration Every Launch:**
  - Every launch and camera switch runs a guided 5-second empty-room calibration automatically once streaming while disarmed, with **Restart 5s**, **Skip for now**, and **Recalibrate** controls.
- **Native App Icons on Every Platform:**
  - Pure-Rust COFF `.rsrc` icon and version metadata embedding on Windows (`build.rs`), rebuilt multi-size `.icns` and clean `.dmg` staging on macOS, and desktop launcher/icon integration on Linux.

### Bug Fixes

- **Fixed post-bell-cutoff Entry Delay across all panels:** Opening an Entry/Exit delay door after an earlier alarm's bell cut-off expired now starts Entry Delay After Alarm instead of firing an immediate burglary alarm, and swinger re-triggering works properly after bell cut-off.
- **Fixed Stay / Away arming & false "Zone Bypassed" display:** Stay automatic interior bypasses are kept separate from manual user bypasses so VistaGuard 20P shows `** ALL SECURE **` unless you manually bypassed a zone, and Away arming behavior matches each panel's rules reliably.
- **Fixed unexpected instant alarms & camera races:** Tightened limb checks against lighting shifts, added Delay-first zone settling to prevent Interior zones racing ahead of doorways, separated camera fault troubles from tamper alarms, and shortened camera zone release hold to 1.0 s.

### Home Assistant alarm notifications and audio output fixes

- **Home Assistant now always knows RoomGuard's state.** RoomGuard fires a `roomguard_state` event on every change, using Home Assistant's own alarm panel words (`disarmed`, `arming`, `armed_away`, `armed_home`, `armed_night`, `pending`, `triggered`), and a `roomguard_alarm` event when an alarm starts or clears, even with no panel linked.
- **Alarm notifications.** Each alarm shows a notification in Home Assistant, removed when RoomGuard is disarmed, and can also send an urgent phone alert through any notify service (Settings, Home Assistant, Alarm notifications). Duress and silent alarms send the event only.
- **Nothing is lost during an outage.** After a reconnect, RoomGuard sends its state again, announces an alarm that began while Home Assistant was offline, and clears one that ended.
- **A Home Assistant URL typed without `http://` works.** It used to crash the Home Assistant connection silently, so RoomGuard never connected or retried.
- **A separate siren speaker gets every command it needs.** Clearing the siren and the wake-up nudge now reach every output, not only the keypad's.
- **An output can no longer go permanently silent.** If an internal error hits while a sound command is applied, that output's synth restarts in place. Before, the stream was rebuilt and the same command replayed into it, failing again every time.
- **A live level meter for each output** in Settings, Sound shows whether RoomGuard is sending sound to that speaker, which tells a RoomGuard problem apart from a Windows volume one.


### Home Assistant reliability

- A Home Assistant restart no longer sets off trouble beeping. Each sensor keeps its last known state for up to 2 minutes (the linked panel used to fault after 8 seconds), and only an outage that outlasts that becomes a supervision trouble, which clears by itself once the sensor reports again.
- Alarms that start and end while Home Assistant is offline are reported when it reconnects, as a `roomguard_alarm` event with `while_offline: true`, a notification and a phone alert.
- New `roomguard_trouble` event whenever RoomGuard's troubles change.
- When Home Assistant refuses an announcement (events need an administrator's token, or a notify service is misspelled), RoomGuard now says so in Settings and logs it once, instead of failing silently.
- The test lab package has no alarm code anymore. A code there made Home Assistant refuse disarming from RoomGuard whenever the codes differed, and Two-Way sync then re-armed RoomGuard.

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
